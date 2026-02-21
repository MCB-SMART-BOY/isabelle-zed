# Official Zed Extension Submission Guide

This guide is for submitting this project to the official Zed extension registry in:

- https://github.com/zed-industries/extensions

## 1. Pre-check this repository

Run from this repository root:

```bash
make zed-official-check
```

This validates the extension ID and license files, and prints the expected
`extensions.toml` snippet.

## 2. Fork and clone the official registry repository

```bash
git clone https://github.com/<your-github-user>/extensions.git zed-extensions
cd zed-extensions
git remote add upstream https://github.com/zed-industries/extensions.git
```

## 3. Create a feature branch

```bash
git checkout -b add-isabelle-extension
```

## 4. Add this repository as a submodule

```bash
git submodule add https://github.com/MCB-SMART-BOY/isabelle-zed.git extensions/isabelle
```

## 5. Add registry entry in `extensions.toml`

Add this entry:

```toml
[isabelle]
submodule = "extensions/isabelle"
path = "zed-extension"
version = "0.2.0"
```

If needed, run sorting in the registry repo:

```bash
pnpm install
pnpm sort-extensions
```

## 6. Commit and push

```bash
git add .gitmodules extensions/isabelle extensions.toml
git commit -m "Add Isabelle extension"
git push origin add-isabelle-extension
```

## 7. Open PR to `zed-industries/extensions`

Open a PR from your fork branch into `zed-industries/extensions:main`.

In the PR description include:

- Extension repository URL.
- Extension scope (`path = "zed-extension"`).
- Runtime mode note: native mode uses `isabelle vscode_server`.

## 8. Updates after first merge

For future releases:

1. Bump `zed-extension/extension.toml` version in this repository.
2. Tag/release if desired.
3. In your fork of `zed-industries/extensions`, update:
   - `extensions/isabelle` submodule commit
   - `[isabelle]` version in `extensions.toml`
4. Open update PR.

## Notes

- Keep extension ID stable as `isabelle`.
- Do not rename ID once published in official registry.
- Keep accepted license files in both repository root and `zed-extension/`.
