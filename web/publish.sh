#!/bin/sh
# Builds the browser version from the last commit and publishes it on GitHub Pages, then waits
# until the page is live and checks that the live file is the one built.
#
# Usage: web/publish.sh [--dry-run] [--allow-dirty]
#   --dry-run      build and prepare the Pages commit, but push nothing
#   --allow-dirty  go on although tracked files have uncommitted changes (they are NOT published)
# Environment: PAGES_REPO (default rabalyn/orbis), PAGES_BRANCH (default main).
# Needs: what web/build.sh needs, git, gh (logged in, allowed to push to PAGES_REPO), curl.
#
# What gets published is always HEAD, built in a clean checkout, so commit first.
# config/settings.json is ignored in the check: the program saves its options there, and the
# build does not read it.
#
# Exit status 0: published, or the live page already holds this build. The last line says which.
# Anything else: nothing was published, or the deploy failed; the message says where.
set -eu

pages_repo=${PAGES_REPO:-rabalyn/orbis}
pages_branch=${PAGES_BRANCH:-main}
dry_run=0
allow_dirty=0
for arg in "$@"; do
    case $arg in
        --dry-run) dry_run=1 ;;
        --allow-dirty) allow_dirty=1 ;;
        -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "publish: unknown option $arg" >&2; exit 2 ;;
    esac
done

say() { echo "publish: $*"; }
fail() { echo "publish: FAILED: $*" >&2; exit 1; }

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
for tool in git gh curl cargo wasm-bindgen python3; do
    command -v "$tool" >/dev/null || fail "$tool is not installed"
done
gh auth status >/dev/null 2>&1 || fail "gh is not logged in (run: gh auth login)"

dirty=$(git status --porcelain --untracked-files=no | grep -v ' config/settings.json$' || true)
if [ -n "$dirty" ]; then
    if [ "$allow_dirty" = 1 ]; then
        say "uncommitted changes are left out of the build:"
        echo "$dirty"
    else
        echo "$dirty" >&2
        fail "tracked files have uncommitted changes; commit them first (or pass --allow-dirty)"
    fi
fi

rev=$(git rev-parse --short HEAD)
subject=$(git log -1 --format=%s)
work=$(mktemp -d "${TMPDIR:-/tmp}/orbis-publish.XXXXXX")
cleanup() {
    git -C "$root" worktree remove --force "$work/src" >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

say "building $rev ($subject) in a clean checkout"
git worktree add --quiet --detach "$work/src" HEAD
# A target folder of its own, so that this build and the one in the checkout do not
# rebuild each other.
CARGO_TARGET_DIR="$root/target/publish" sh "$work/src/web/build.sh" "$work/index.html"
mkdir -p dist
cp "$work/index.html" dist/index.html
built=$(sha256sum "$work/index.html" | cut -d' ' -f1)

say "fetching $pages_repo ($pages_branch)"
gh repo clone "$pages_repo" "$work/pages" -- --quiet --depth 1 --branch "$pages_branch"
cd "$work/pages"
if [ -f index.html ] && [ "$(sha256sum index.html | cut -d' ' -f1)" = "$built" ]; then
    say "up to date: $pages_repo already holds this build of $rev"
    exit 0
fi
cp "$work/index.html" index.html
touch .nojekyll
git add index.html .nojekyll
git commit --quiet -m "$subject" -m "Built from kugelblitz-reimpl $rev."

if [ "$dry_run" = 1 ]; then
    git --no-pager log --stat -1
    say "dry run: built $rev, nothing pushed"
    exit 0
fi

git push --quiet origin "HEAD:$pages_branch"
sha=$(git rev-parse HEAD)
say "pushed $(git rev-parse --short HEAD) to $pages_repo; waiting for the Pages deploy"

# Pages builds on its own after a push; wait up to 10 minutes for it to finish.
status=""
i=0
while [ $i -lt 60 ]; do
    line=$(gh api "repos/$pages_repo/pages/builds/latest" --jq '.status + " " + .commit' 2>/dev/null || true)
    status=${line%% *}
    if [ "${line#* }" = "$sha" ]; then
        case $status in
            built) break ;;
            errored) fail "the Pages deploy of $sha errored; see https://github.com/$pages_repo/actions" ;;
        esac
    fi
    i=$((i + 1))
    sleep 10
done
[ "$status" = built ] || fail "the Pages deploy did not finish within 10 minutes (last status: ${status:-none})"

url=$(gh api "repos/$pages_repo/pages" --jq .html_url)
# The CDN can serve the old file for a little while after the deploy.
i=0
while [ $i -lt 30 ]; do
    live=$(curl -fsSL -H 'Cache-Control: no-cache' "$url" | sha256sum | cut -d' ' -f1)
    if [ "$live" = "$built" ]; then
        say "published: $url serves the build of $rev"
        exit 0
    fi
    i=$((i + 1))
    sleep 10
done
fail "the deploy finished, but $url still serves another file after 5 minutes"
