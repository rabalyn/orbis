# Release and web publishing

Orbis uses [Semantic Versioning](https://semver.org/). `Cargo.toml` is the canonical source of the app version. Every web
publish must come from a clean commit with a new SemVer version and a matching annotated Git tag named `v<version>`.
The app's technical panel (F12) shows the package version, the 12-character Git commit used as the build ID, and the exact
release tag, or `untagged` for development builds.

Every release has notes at `docs/releases/v<version>.md`. The same notes are published as the GitHub Release for that tag.
Do not publish a web build without both the matching tag and non-empty release notes.

## Release workflow

1. Make and review the release changes. Update user-facing documentation and source credits where needed.
2. Bump `package.version` in `Cargo.toml` using SemVer. Use a prerelease suffix for preview releases, such as `0.2.0-rc.1`.
3. Add `docs/releases/v<version>.md` with the user-visible changes, notable fixes, and known limitations. Keep the notes
   concise and specific to this release.
4. Commit the release contents, including the version bump and notes. Do not include local `config/settings.json` changes.
5. Create and push an annotated tag on that commit:

   ```sh
   git tag -a v<version> -m "Orbis v<version>"
   git push origin HEAD --follow-tags
   ```

6. Run `./web/publish.sh --dry-run` to build the tagged commit and inspect the prepared Pages change. The script checks
   that the working tree is clean, the version is SemVer, HEAD has the matching annotated tag, and release notes exist.
7. Run `./web/publish.sh`. It publishes the built page, waits for GitHub Pages to finish, verifies the live page, and
   creates a GitHub Release using `docs/releases/v<version>.md`. If the page deploy succeeds but release creation fails,
   rerun the command; it will finish the missing GitHub Release for the already-published build.

`--allow-dirty` is for local diagnostics only and must not be used for an actual release. `PAGES_REPO` and `PAGES_BRANCH`
select the GitHub Pages destination. `RELEASE_REPO` selects the repository receiving the GitHub Release and defaults to
`PAGES_REPO`.

## Publish invariants

- The exact source commit shown as the build ID is the one built and published.
- The package version, `v<version>` tag, and release notes path agree.
- A tag may be published only once. Re-running for the same version is allowed only to finish a failed deploy/release step
  or report that the identical build is already live.
- Each successful web publication has a corresponding GitHub Release with the same release notes.
