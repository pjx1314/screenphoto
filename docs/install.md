# 安装说明

收到安装请求时，按顺序执行以下步骤，完成后向用户报告安装位置和验证结果。仓库或 Release 不存在，或选定资产的下载地址无效时，立即停止并如实报告，不得猜测下载地址。

## 1. 选择平台包

按实际执行 `screenphoto` 命令的系统和架构选择资产。

- Linux `x86_64`：`screenphoto-linux-x86_64.zip`
- Windows `x86_64`：`screenphoto-windows-x86_64.zip`

解压后的目录结构如下：

```text
screenphoto/
  SKILL.md
  LICENSE
  bin/screenphoto          # Linux
  # Windows 下为 bin/screenphoto.exe
```

当前 Release 没有对应平台的预编译包时，按 [从源码构建](#从源码构建) 处理。该节未覆盖的平台停止并报告。

## 2. 安装技能目录

### 下载 Release 资产

1. 从 [Releases](https://github.com/pjx1314/screenphoto/releases) 下载对应平台的 ZIP 和同一版本的 `SHA256SUMS`。
2. 核对 ZIP 的 SHA-256 摘要与 `SHA256SUMS` 一致。
3. 解压并将完整的 `screenphoto/` 目录放入当前客户端的技能目录，保持 `SKILL.md` 与 `bin/` 的相对位置不变。
4. Linux 环境下进入技能目录，为可执行文件添加执行权限：

```bash
chmod +x bin/screenphoto
```

### 使用 skills 命令行

[Vercel skills](https://github.com/vercel-labs/skills) 需要 Node.js 22.20 以上版本。使用平台 ZIP 直链可将 `SKILL.md` 与 `bin/` 内的可执行文件一并装入技能目录：

```bash
# Linux
npx skills add https://github.com/pjx1314/screenphoto/releases/latest/download/screenphoto-linux-x86_64.zip --skill screenphoto

# Windows
npx skills add https://github.com/pjx1314/screenphoto/releases/latest/download/screenphoto-windows-x86_64.zip --skill screenphoto
```

注意：`skills add` 默认安装到当前项目，全局安装加 `-g`，不会把 `screenphoto` 加入系统 `PATH`。`skills` 解压 `.zip` 归档时不保留 Unix 执行权限位。项目内多 Agent 软链安装的实际目录是 `.agents/skills/screenphoto`，单 Agent 或全局安装以 `skills` 输出的实际目录为准。Linux 安装后进入实际技能目录执行 `chmod +x bin/screenphoto`。

## 3. 验证安装

1. 在技能目录运行 `bin/screenphoto --version`，Windows 环境运行 `bin\screenphoto.exe --version`，确认正常输出版本号。
2. 准备一张可读的 PNG 或 JPEG 截图，指定独立的输出路径运行转换：

```bash
bin/screenphoto shot /path/to/sample.png -o /path/to/check-photo.jpg --json
```

3. 核对退出码为 `0`、JSON 中 `ok` 为 `true`，同时确认 `/path/to/check-photo.jpg` 存在且文件字节数等于 `bytes`。验证通过后删除测试输出文件，技能即可投入使用。

## 从源码构建

本地构建需要 Rust 1.88 以上版本和 Cargo。在仓库根目录运行打包脚本，生成完整的技能目录、平台 ZIP 和 `SHA256SUMS`。不带目标参数时按当前主机目标构建，产物位于 `dist/<主机目标>/screenphoto`。

Linux musl 静态构建：

```bash
rustup target add x86_64-unknown-linux-musl
bash scripts/package.sh dist x86_64-unknown-linux-musl
```

生成的技能目录位于 `dist/x86_64-unknown-linux-musl/screenphoto`。

Windows 构建需要 MSVC C++ 生成工具，在 PowerShell 中执行：

```powershell
rustup target add x86_64-pc-windows-msvc
.\scripts\package.ps1 -Out dist -Target x86_64-pc-windows-msvc
```

生成的技能目录位于 `dist/x86_64-pc-windows-msvc/screenphoto`。将该目录复制到技能目录，或传给 `npx skills add <本地目录> --skill screenphoto`，再按 [3. 验证安装](#3-验证安装) 检查。
