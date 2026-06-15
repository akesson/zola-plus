# CLAUDE.md

This is **zola-plus**, a fork of [getzola/zola](https://github.com/getzola/zola).

## Branching strategy

- **`master`** mirrors `upstream/master` (getzola/zola). Keep it clean — no
  zola-plus customizations land here. It exists only to track and pull in
  upstream updates:

  ```sh
  git checkout master
  git fetch upstream
  git merge --ff-only upstream/master   # or: git rebase upstream/master
  ```

- **`zola-plus-main`** is the working branch. All zola-plus customizations
  live here (e.g. the binary rename from `zola` to `zola-plus`). Branch new
  work off this, and integrate upstream by merging/rebasing `master` into it
  after syncing:

  ```sh
  git checkout zola-plus-main
  git merge master            # or: git rebase master
  ```

Remotes: `origin` → akesson/zola-plus, `upstream` → getzola/zola.
