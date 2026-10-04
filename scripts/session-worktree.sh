#!/bin/sh
# Per-session git worktrees.
#
# Parallel agent sessions sharing one checkout can wipe each other's
# uncommitted files with a `checkout` or `reset --hard`. Give each task its own
# worktree under .worktrees/ (git-ignored) instead, then remove it again so the
# directories never pile up.
#
#   sh scripts/session-worktree.sh start feat/my-change   # prints the path
#   cd .worktrees/feat-my-change                          # work + commit there
#   sh scripts/session-worktree.sh finish feat/my-change  # remove + prune
#
# Nothing is cleaned up automatically: run `finish` when the task is done or
# abandoned. `list` shows what is left, `prune` clears stale metadata.
set -eu

root=$(git rev-parse --show-toplevel)
default_base=origin/main

usage() {
    cat <<'EOF'
usage: sh scripts/session-worktree.sh <command>

  start <branch> [base]      create .worktrees/<branch> from base (default origin/main)
  finish <branch> [--delete] remove the worktree (and its branch with --delete)
  list                       list worktrees
  prune                      drop stale worktree metadata
EOF
}

# "feat/x" -> "feat-x", so the directory is a plain name under .worktrees/.
slug() {
    printf '%s' "$1" | tr '/\\' '--'
}

worktree_path() {
    printf '%s/.worktrees/%s' "$root" "$(slug "$1")"
}

cmd=${1:-}
case "$cmd" in
start)
    branch=${2:?a branch name is required}
    base=${3:-$default_base}
    path=$(worktree_path "$branch")

    # Reuse an existing worktree for this branch instead of stacking copies.
    if git -C "$root" worktree list --porcelain | grep -qx "worktree $path"; then
        printf '%s\n' "$path"
        exit 0
    fi

    git -C "$root" fetch -q origin || true
    if git -C "$root" show-ref --verify --quiet "refs/heads/$branch"; then
        git -C "$root" worktree add -q "$path" "$branch"
    else
        git -C "$root" worktree add -q -b "$branch" "$path" "$base"
    fi
    printf '%s\n' "$path"
    ;;
finish)
    branch=${2:?a branch name is required}
    path=$(worktree_path "$branch")
    if [ -d "$path" ]; then
        # Refuses when the worktree has uncommitted or untracked changes, so
        # `finish` cannot silently discard work: commit, or remove it by hand.
        git -C "$root" worktree remove "$path"
    fi
    git -C "$root" worktree prune
    if [ "${3:-}" = "--delete" ]; then
        git -C "$root" branch -D "$branch" 2>/dev/null || true
    fi
    printf 'removed %s\n' "$path"
    ;;
list)
    git -C "$root" worktree list
    ;;
prune)
    git -C "$root" worktree prune
    ;;
*)
    usage
    exit 2
    ;;
esac
