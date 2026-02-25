# Official Zed Extension Submission Guide / 官方 Zed 扩展提交流程

## 中文（简体）

本文档用于将本项目提交到官方扩展仓库：

- https://github.com/zed-industries/extensions

### 1. 在当前仓库执行预检查

在 `<repo-root>` 执行：

```bash
make zed-official-check
```

该命令会检查：扩展 ID、许可证文件，以及官方仓库中是否已存在同名 ID。

### 2. Fork 并克隆官方扩展仓库

```bash
git clone https://github.com/<your-github-user>/extensions.git zed-extensions
cd zed-extensions
git remote add upstream https://github.com/zed-industries/extensions.git
```

### 3. 创建功能分支

```bash
git checkout -b add-isabelle-extension
```

### 4. 添加本仓库为子模块

```bash
git submodule add https://github.com/MCB-SMART-BOY/isabelle-zed.git extensions/isabelle
```

如果你的仓库地址不同，请替换成你自己的仓库 URL。

### 5. 在 `extensions.toml` 增加条目

```toml
[isabelle]
submodule = "extensions/isabelle"
path = "zed-extension"
version = "<read-from-zed-extension/extension.toml>"
```

需要排序时：

```bash
pnpm install
pnpm sort-extensions
```

### 6. 提交并推送

```bash
git add .gitmodules extensions/isabelle extensions.toml
git commit -m "Add Isabelle extension"
git push origin add-isabelle-extension
```

### 7. 向 `zed-industries/extensions` 发 PR

PR 描述建议包含：

- 扩展仓库地址
- `path = "zed-extension"` 说明
- 运行模式说明：默认 native 模式依赖 `isabelle vscode_server`

### 8. 首次合并后的版本更新流程

1. 在本仓库更新 `zed-extension/extension.toml` 的 `version`
2. 更新 `CHANGELOG.md`
3. 可选：打 tag / 发 release
4. 通过 `make release-package` 生成发布包（包含 `LICENSE` 与 `docs/CHANGELOG.md`）
5. 在你的 `extensions` fork 中更新：
   - `extensions/isabelle` 子模块提交
   - `extensions.toml` 里的 `[isabelle]` 版本
6. 再发一次更新 PR

### 注意事项

- 扩展 ID 固定为 `isabelle`
- 发布后不要改扩展 ID
- 保持仓库根目录与 `zed-extension/` 下都有合法许可证文件

## English

Use this guide to submit this project to the official Zed extension registry:

- https://github.com/zed-industries/extensions

### 1. Run pre-check in this repository

From `<repo-root>`:

```bash
make zed-official-check
```

This checks extension ID rules, license files, and duplicate ID against the official registry.

### 2. Fork and clone the official registry repository

```bash
git clone https://github.com/<your-github-user>/extensions.git zed-extensions
cd zed-extensions
git remote add upstream https://github.com/zed-industries/extensions.git
```

### 3. Create a feature branch

```bash
git checkout -b add-isabelle-extension
```

### 4. Add this repository as a submodule

```bash
git submodule add https://github.com/MCB-SMART-BOY/isabelle-zed.git extensions/isabelle
```

Replace the URL if your extension repository lives elsewhere.

### 5. Add entry to `extensions.toml`

```toml
[isabelle]
submodule = "extensions/isabelle"
path = "zed-extension"
version = "<read-from-zed-extension/extension.toml>"
```

If sorting is needed:

```bash
pnpm install
pnpm sort-extensions
```

### 6. Commit and push

```bash
git add .gitmodules extensions/isabelle extensions.toml
git commit -m "Add Isabelle extension"
git push origin add-isabelle-extension
```

### 7. Open PR to `zed-industries/extensions`

Recommended PR notes:

- Extension repository URL
- Scope note: `path = "zed-extension"`
- Runtime note: default native mode depends on `isabelle vscode_server`

### 8. Update workflow after first merge

1. Bump `version` in `zed-extension/extension.toml` in this repository
2. Update `CHANGELOG.md`
3. Optional: create tag/release
4. Build release package via `make release-package` (includes `LICENSE` and `docs/CHANGELOG.md`)
5. In your fork of `extensions`, update:
   - `extensions/isabelle` submodule commit
   - `[isabelle]` version in `extensions.toml`
6. Open an update PR

### Notes

- Keep extension ID stable as `isabelle`
- Do not rename the extension ID after publishing
- Keep valid license files in both repository root and `zed-extension/`
