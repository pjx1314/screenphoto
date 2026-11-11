# 发布流程

仓库地址为 [pjx1314/screenphoto](https://github.com/pjx1314/screenphoto)。持续集成和自动发布定义在 `.github/workflows/` 下，编译与打包产物不入库。

## 忽略项

`.gitignore` 排除以下本地构建产物：

- `target/`：Cargo 编译缓存与二进制产物。
- `dist/`：本地技能包目录、ZIP 和校验文件。
- `out/`：本地调试输出图。
- `*.crate`：本地打包生成的源码归档。

## 持续集成

`.github/workflows/ci.yml` 在分支推送、Pull Request 和手动触发时运行，同时作为可复用工作流供发布流程调用。检查内容包括：

- Linux 与 Windows：`cargo fmt --all -- --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`cargo test --locked --all-features`。
- Linux 附加项：`--no-default-features` 纯库编译、`native_api` 测试、`verify-corpus` 固定测试集校验和 `cargo package --locked` 源码包校验。

任一检查失败或取消时，后续发布任务不会执行。

## 标签发布

发版前确认三处版本号一致：`Cargo.toml` 的 `version`、`SKILL.md` 头部的 `metadata.version`，以及待推送的 Git 标签名。同时确认 GitHub 仓库简介与 `Cargo.toml` 的 `description` 一致。

推送 `v*` 标签触发 `.github/workflows/release.yml`：

```bash
git tag v0.1.0
git push origin v0.1.0
```

工作流分四个任务执行：

1. `checks`：完整调用 `ci.yml`。
2. `build`：在 `ubuntu-latest` 上安装 `musl-tools`，运行 `scripts/package.sh` 打包，再由后续步骤复制为 `screenphoto-linux-x86_64.zip`。在 `windows-latest` 上运行 `scripts/package.ps1` 打包，同样复制为 `screenphoto-windows-x86_64.zip`。
3. `crate`：在 `ubuntu-latest` 上执行 `cargo package --locked`，收集 `screenphoto-<version>.crate`。
4. `release`：仅在 `push` 事件且标签以 `v` 开头时运行，校验标签名等于 `v<Cargo.toml version>`，汇总生成 `SHA256SUMS`，再创建或更新同名 GitHub Release。

发布的 Release 资产固定为 4 个文件：

- `screenphoto-linux-x86_64.zip`
- `screenphoto-windows-x86_64.zip`
- `screenphoto-<version>.crate`
- `SHA256SUMS`

手动通过 `workflow_dispatch` 运行 `release.yml` 时，工作流只执行校验并上传 Actions 构建产物，不会创建或修改 GitHub Release。

## 本地打包

在仓库根目录执行打包脚本，默认输出到 `dist/`：

```bash
# Linux / macOS
bash scripts/package.sh dist

# Windows PowerShell
.\scripts\package.ps1 -Out dist
```

打包命令会自动核对 `SKILL.md` 中的 `metadata.version` 是否与 `Cargo.toml` 一致。需要校验 Agent Skills 规范头信息时，可按 [skills-ref 说明](https://github.com/agentskills/agentskills/tree/main/skills-ref#installation)安装官方参考工具后运行 `skills-ref validate dist/<target>/screenphoto`。
