import stat

from branchless.init import (
    _UPDATE_MARKER_END,
    _UPDATE_MARKER_START,
    _update_between_lines,
)
from helpers import Git


def test_update_between_lines() -> None:
    assert _update_between_lines(
        [
            "hello, world\n",
            _UPDATE_MARKER_START,
            "contents 1\n",
            _UPDATE_MARKER_END,
            "goodbye, world\n",
        ],
        ["contents 2\n"],
    ) == [
        "hello, world\n",
        _UPDATE_MARKER_START,
        "contents 2\n",
        _UPDATE_MARKER_END,
        "goodbye, world\n",
    ]


def test_hook_installed(git: Git) -> None:
    git.init_repo()
    hook_path = git.path / ".git" / "hooks" / "post-commit"
    assert hook_path.exists()
    assert hook_path.stat().mode & stat.S_IXUSR
    assert hook_path.stat().mode & stat.S_IXGRP
    assert hook_path.stat().mode & stat.S_IXOTH


def test_alias_installed(git: Git) -> None:
    git.init_repo()
    assert (
        git.run("smartlog")
        == """\
@ f777ecc9 (master) create initial.txt
"""
    )
    assert (
        git.run("sl")
        == """\
@ f777ecc9 (master) create initial.txt
"""
    )
