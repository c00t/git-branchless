//! Convenience commands to help the user move through a stack of commits.

use std::collections::HashSet;

use std::ffi::OsString;
use std::fmt::Write;
use std::time::SystemTime;

use cursive::theme::BaseColor;
use cursive::utils::markup::StyledString;
use eden_dag::DagAlgorithm;
use tracing::{instrument, warn};

use crate::commands::smartlog::make_smartlog_graph;
use crate::opts::{CheckoutOptions, TraverseCommitsOptions};
use crate::tui::prompt_select_commit;
use lib::core::config::get_next_interactive;
use lib::core::dag::{sort_commit_set, CommitSet, Dag};
use lib::core::effects::Effects;
use lib::core::eventlog::{EventLogDb, EventReplayer};
use lib::core::formatting::{printable_styled_string, Pluralize};
use lib::core::node_descriptors::{
    BranchesDescriptor, CommitMessageDescriptor, CommitOidDescriptor,
    DifferentialRevisionDescriptor, NodeDescriptor, Redactor, RelativeTimeDescriptor,
};
use lib::git::{check_out_commit, CheckOutCommitOptions, GitRunInfo, NonZeroOid, Repo};

/// The command being invoked, indicating which direction to traverse commits.
#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Traverse child commits.
    Next,

    /// Traverse parent commits.
    Prev,
}

/// The number of commits to traverse.
#[derive(Clone, Copy, Debug)]
pub enum Distance {
    /// Traverse this number of commits or branches.
    NumCommits {
        /// The number of commits or branches to traverse.
        amount: usize,

        /// If `true`, count the number of branches traversed, not commits.
        move_by_branches: bool,
    },

    /// Traverse as many commits as possible.
    AllTheWay {
        /// If `true`, find the farthest commit with a branch attached to it.
        move_by_branches: bool,
    },
}

/// Some commits have multiple children, which makes `next` ambiguous. These
/// values disambiguate which child commit to go to, according to the committed
/// date.
#[derive(Clone, Copy, Debug)]
pub enum Towards {
    /// When encountering multiple children, select the newest one.
    Newest,

    /// When encountering multiple children, select the oldest one.
    Oldest,

    /// When encountering multiple children, interactively prompt for
    /// which one to advance to.
    Interactive,
}

#[instrument(skip(commit_descriptors))]
fn advance(
    effects: &Effects,
    repo: &Repo,
    dag: &Dag,
    commit_descriptors: &mut [&mut dyn NodeDescriptor],
    current_oid: NonZeroOid,
    command: Command,
    distance: Distance,
    towards: Option<Towards>,
) -> eyre::Result<Option<NonZeroOid>> {
    let towards = match towards {
        Some(towards) => Some(towards),
        None => {
            if get_next_interactive(repo)? {
                Some(Towards::Interactive)
            } else {
                None
            }
        }
    };

    let public_commits = dag.query().ancestors(dag.main_branch_commit.clone())?;

    let glyphs = effects.get_glyphs();
    let mut current_oid = current_oid;
    let mut i = 0;
    loop {
        let candidate_commits = match command {
            Command::Next => {
                let child_commits = || -> eyre::Result<CommitSet> {
                    let result = dag
                        .query()
                        .children(CommitSet::from(current_oid))?
                        .difference(&dag.obsolete_commits);
                    Ok(result)
                };

                let descendant_branches = || -> eyre::Result<CommitSet> {
                    let descendant_commits = dag.query().descendants(child_commits()?)?;
                    let descendant_branches = dag.branch_commits.intersection(&descendant_commits);
                    let descendants = dag.query().descendants(descendant_branches)?;
                    let nearest_descendant_branches = dag.query().roots(descendants)?;
                    Ok(nearest_descendant_branches)
                };

                let children = match distance {
                    Distance::AllTheWay {
                        move_by_branches: false,
                    }
                    | Distance::NumCommits {
                        amount: _,
                        move_by_branches: false,
                    } => child_commits()?,

                    Distance::AllTheWay {
                        move_by_branches: true,
                    }
                    | Distance::NumCommits {
                        amount: _,
                        move_by_branches: true,
                    } => descendant_branches()?,
                };

                sort_commit_set(repo, dag, &children)?
            }

            Command::Prev => {
                let parent_commits = || -> eyre::Result<CommitSet> {
                    let result = dag.query().parents(CommitSet::from(current_oid))?;
                    Ok(result)
                };
                let ancestor_branches = || -> eyre::Result<CommitSet> {
                    let ancestor_commits = dag.query().ancestors(parent_commits()?)?;
                    let ancestor_branches = dag.branch_commits.intersection(&ancestor_commits);
                    let nearest_ancestor_branches =
                        dag.query().heads_ancestors(ancestor_branches)?;
                    Ok(nearest_ancestor_branches)
                };

                let parents = match distance {
                    Distance::AllTheWay {
                        move_by_branches: false,
                    } => {
                        // The `--all` flag for `git prev` isn't useful if all it does
                        // is take you to the root commit for the repository.  Instead,
                        // we assume that the user wanted to get to the root commit for
                        // their current *commit stack*. We filter out commits which
                        // aren't part of the commit stack so that we stop early here.
                        let parents = parent_commits()?;
                        parents.difference(&public_commits)
                    }

                    Distance::AllTheWay {
                        move_by_branches: true,
                    } => {
                        // See above case.
                        let parents = ancestor_branches()?;
                        parents.difference(&public_commits)
                    }

                    Distance::NumCommits {
                        amount: _,
                        move_by_branches: false,
                    } => parent_commits()?,

                    Distance::NumCommits {
                        amount: _,
                        move_by_branches: true,
                    } => ancestor_branches()?,
                };

                sort_commit_set(repo, dag, &parents)?
            }
        };

        match distance {
            Distance::NumCommits {
                amount,
                move_by_branches: _,
            } => {
                if i == amount {
                    break;
                }
            }

            Distance::AllTheWay {
                move_by_branches: _,
            } => {
                if candidate_commits.is_empty() {
                    break;
                }
            }
        }

        let pluralize = match command {
            Command::Next => Pluralize {
                determiner: None,
                amount: i,
                unit: ("child", "children"),
            },

            Command::Prev => Pluralize {
                determiner: None,
                amount: i,
                unit: ("parent", "parents"),
            },
        };
        let header = format!(
            "Found multiple possible {} commits to go to after traversing {}:",
            pluralize.unit.0, pluralize,
        );

        current_oid = match (towards, candidate_commits.as_slice()) {
            (_, []) => {
                writeln!(
                    effects.get_output_stream(),
                    "{}",
                    printable_styled_string(
                        glyphs,
                        StyledString::styled(
                            format!(
                                "No more {} commits to go to after traversing {}.",
                                pluralize.unit.0, pluralize,
                            ),
                            BaseColor::Yellow.light()
                        )
                    )?
                )?;

                if i == 0 {
                    // If we didn't succeed in traversing any commits, then
                    // treat the operation as a failure. Otherwise, assume that
                    // the user just meant to go as many commits as possible.
                    return Ok(None);
                } else {
                    break;
                }
            }

            (_, [only_child]) => only_child.get_oid(),
            (Some(Towards::Newest), [.., newest_child]) => newest_child.get_oid(),
            (Some(Towards::Oldest), [oldest_child, ..]) => oldest_child.get_oid(),
            (Some(Towards::Interactive), [_, _, ..]) => {
                match prompt_select_commit(
                    Some(&header),
                    "",
                    candidate_commits,
                    commit_descriptors,
                )? {
                    Some(oid) => oid,
                    None => {
                        return Ok(None);
                    }
                }
            }
            (None, [_, _, ..]) => {
                writeln!(effects.get_output_stream(), "{}", header)?;
                for (j, child) in (0..).zip(candidate_commits.iter()) {
                    let descriptor = if j == 0 {
                        " (oldest)"
                    } else if j + 1 == candidate_commits.len() {
                        " (newest)"
                    } else {
                        ""
                    };

                    writeln!(
                        effects.get_output_stream(),
                        "  {} {}{}",
                        glyphs.bullet_point,
                        printable_styled_string(glyphs, child.friendly_describe(glyphs)?)?,
                        descriptor
                    )?;
                }
                writeln!(effects.get_output_stream(), "(Pass --oldest (-o), --newest (-n), or --interactive (-i) to select between ambiguous commits)")?;
                return Ok(None);
            }
        };

        i += 1;
    }
    Ok(Some(current_oid))
}

/// Go forward or backward a certain number of commits.
#[instrument]
pub fn traverse_commits(
    effects: &Effects,
    git_run_info: &GitRunInfo,
    command: Command,
    options: &TraverseCommitsOptions,
) -> eyre::Result<isize> {
    let TraverseCommitsOptions {
        num_commits,
        all_the_way,
        move_by_branches,
        oldest,
        newest,
        interactive,
        merge,
        force,
    } = *options;

    let distance = match (all_the_way, num_commits) {
        (false, None) => Distance::NumCommits {
            amount: 1,
            move_by_branches,
        },

        (false, Some(amount)) => Distance::NumCommits {
            amount,
            move_by_branches,
        },

        (true, None) => Distance::AllTheWay { move_by_branches },

        (true, Some(_)) => {
            eyre::bail!("num_commits and --all cannot both be set")
        }
    };

    let towards = match (oldest, newest, interactive) {
        (false, false, false) => None,
        (true, false, false) => Some(Towards::Oldest),
        (false, true, false) => Some(Towards::Newest),
        (false, false, true) => Some(Towards::Interactive),
        (_, _, _) => {
            eyre::bail!("Only one of --oldest, --newest, and --interactive can be set")
        }
    };

    let repo = Repo::from_current_dir()?;
    let head_info = repo.get_head_info()?;
    let references_snapshot = repo.get_references_snapshot()?;
    let conn = repo.get_db_conn()?;
    let event_log_db = EventLogDb::new(&conn)?;
    let event_replayer = EventReplayer::from_event_log_db(effects, &repo, &event_log_db)?;
    let event_cursor = event_replayer.make_default_cursor();
    let dag = Dag::open_and_sync(
        effects,
        &repo,
        &event_replayer,
        event_cursor,
        &references_snapshot,
    )?;

    let head_oid = match references_snapshot.head_oid {
        Some(head_oid) => head_oid,
        None => {
            eyre::bail!("No HEAD present; cannot calculate next commit");
        }
    };

    let current_oid = advance(
        effects,
        &repo,
        &dag,
        &mut [
            &mut CommitOidDescriptor::new(true)?,
            &mut RelativeTimeDescriptor::new(&repo, SystemTime::now())?,
            &mut BranchesDescriptor::new(
                &repo,
                &head_info,
                &references_snapshot,
                &Redactor::Disabled,
            )?,
            &mut DifferentialRevisionDescriptor::new(&repo, &Redactor::Disabled)?,
            &mut CommitMessageDescriptor::new(&Redactor::Disabled)?,
        ],
        head_oid,
        command,
        distance,
        towards,
    )?;
    let current_oid = match current_oid {
        None => return Ok(1),
        Some(current_oid) => current_oid,
    };

    let current_oid: OsString = match distance {
        Distance::AllTheWay {
            move_by_branches: false,
        }
        | Distance::NumCommits {
            amount: _,
            move_by_branches: false,
        } => current_oid.to_string().into(),

        Distance::AllTheWay {
            move_by_branches: true,
        }
        | Distance::NumCommits {
            amount: _,
            move_by_branches: true,
        } => {
            let empty = HashSet::new();
            let branches = references_snapshot
                .branch_oid_to_names
                .get(&current_oid)
                .unwrap_or(&empty);

            if branches.is_empty() {
                warn!(?current_oid, "No branches attached to commit with OID");
                current_oid.to_string().into()
            } else if branches.len() == 1 {
                let branch = branches.iter().next().unwrap();
                branch.clone()
            } else {
                // It's ambiguous which branch the user wants; just check out the commit directly.
                current_oid.to_string().into()
            }
        }
    };

    let additional_args = {
        let mut args: Vec<OsString> = Vec::new();
        if merge {
            args.push("--merge".into());
        }
        if force {
            args.push("--force".into())
        }
        args
    };
    check_out_commit(
        effects,
        git_run_info,
        None,
        Some(&current_oid),
        &CheckOutCommitOptions {
            additional_args,
            ..Default::default()
        },
    )
}

fn get_initial_query(checkout_options: &CheckoutOptions) -> Option<&str> {
    match checkout_options {
        CheckoutOptions {
            interactive: true,
            target: Some(target),
            branch_name: _,
            force: _,
            merge: _,
        } => Some(target),

        CheckoutOptions {
            interactive: false,
            target: Some(_),
            branch_name: None,
            force: false,
            merge: false,
        } => None,

        CheckoutOptions {
            interactive: true,
            target: None,
            branch_name: _,
            force: _,
            merge: _,
        }
        | CheckoutOptions {
            interactive: false,
            target: None,
            branch_name: None,
            force: false,
            merge: false,
        } => Some(""),

        CheckoutOptions {
            interactive: false,
            target: Some(_),
            branch_name: _,
            force: _,
            merge: _,
        }
        | CheckoutOptions {
            interactive: false,
            target: _,
            branch_name: Some(_),
            force: _,
            merge: _,
        }
        | CheckoutOptions {
            interactive: false,
            target: _,
            branch_name: _,
            force: true,
            merge: _,
        }
        | CheckoutOptions {
            interactive: false,
            target: _,
            branch_name: _,
            force: _,
            merge: true,
        } => None,
    }
}

/// Interactively checkout a commit from the smartlog.
pub fn checkout(
    effects: &Effects,
    git_run_info: &GitRunInfo,
    checkout_options: &CheckoutOptions,
) -> eyre::Result<isize> {
    let CheckoutOptions {
        interactive: _,
        branch_name,
        force,
        merge,
        target,
    } = checkout_options;

    let repo = Repo::from_current_dir()?;
    let head_info = repo.get_head_info()?;
    let references_snapshot = repo.get_references_snapshot()?;
    let conn = repo.get_db_conn()?;
    let event_log_db = EventLogDb::new(&conn)?;
    let event_replayer = EventReplayer::from_event_log_db(effects, &repo, &event_log_db)?;
    let event_cursor = event_replayer.make_default_cursor();
    let dag = Dag::open_and_sync(
        effects,
        &repo,
        &event_replayer,
        event_cursor,
        &references_snapshot,
    )?;

    let graph = make_smartlog_graph(
        effects,
        &repo,
        &dag,
        &event_replayer,
        event_cursor,
        true,
        false,
    )?;

    let initial_query = get_initial_query(checkout_options);
    let target = match initial_query {
        None => target.clone(),
        Some(initial_query) => {
            match prompt_select_commit(
                None,
                initial_query,
                graph.get_commits(),
                &mut [
                    &mut CommitOidDescriptor::new(true)?,
                    &mut RelativeTimeDescriptor::new(&repo, SystemTime::now())?,
                    &mut BranchesDescriptor::new(
                        &repo,
                        &head_info,
                        &references_snapshot,
                        &Redactor::Disabled,
                    )?,
                    &mut DifferentialRevisionDescriptor::new(&repo, &Redactor::Disabled)?,
                    &mut CommitMessageDescriptor::new(&Redactor::Disabled)?,
                ],
            )? {
                Some(oid) => Some(oid.to_string()),
                None => return Ok(1),
            }
        }
    };

    let additional_args = {
        let mut args: Vec<OsString> = Vec::new();
        if let Some(branch_name) = branch_name {
            args.push("-b".into());
            args.push(branch_name.into());
        }
        if *force {
            args.push("-f".into());
        }
        if *merge {
            args.push("-m".into());
        }
        args
    };

    let exit_code = check_out_commit(
        effects,
        git_run_info,
        None,
        target.as_ref(),
        &CheckOutCommitOptions {
            additional_args,
            render_smartlog: true,
        },
    )?;
    Ok(exit_code)
}
