# 测试记录

本文档记录 `0.1.0` 版本的本地测试覆盖、基准耗时和打包验证结果。持续集成的远端执行记录见仓库 GitHub Actions 页面。

## 自动化测试

在 Linux 开发环境运行完整检查套件：

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo check --locked --no-default-features --lib
cargo test --locked --no-default-features --test native_api
cargo run --locked --features dev-tools --bin screenphoto-dev -- verify-corpus
```

格式检查通过，Clippy 零警告，全特性模式下共 65 项测试通过：

- 库单元测试，共 32 项：覆盖图片解码、透明底合成和 EXIF 方向校正。输出尺寸上限、默认斜拍直线保持、受限裁切和采样有效域已包含在内。线性光预缩小、奇数边界和直方图测光的相位无关性各有专门测试。离焦核与屏幕载波频响、零畸变采样连续性、反光分布和随机流复现也都有覆盖。
- 维护工具测试，共 3 项：覆盖耗时百分位插值和 `SKILL.md` 头部版本号解析。该版本号与包版本号的一致性检查也在其中。
- 命令行单元测试，共 19 项：覆盖受限字节读取和 JPEG 头部尺寸提取。临时文件原子替换、失败清理和已有文件保护均有测试。Unix 255 字节文件名、形似临时文件的目标名、原图别名拒绝和默认输出名推导各有回归用例。退出码映射、进程内 `shot` 与 `capture` 流程和抓屏几何校验也包含在内。
- 公共接口测试，共 6 项：验证随机种子复现与 `u64::MAX` 边界、`1920` 长边上限，以及 RGB8 与 PNG 路径结果一致。中央 90% 主体保留、单像素与奇数尺寸输入、损坏输入拦截也在检查范围内。该测试集在 `--no-default-features` 下同样全部通过。
- 命令行进程测试，共 5 项：启动子进程验证原图防覆盖和损坏输入保护，确认非法参数返回退出码 `2`。成功 JSON 与写入文件的字节数一致，Unix 下非 UTF-8 源文件名的默认输出不冲突。

两份 GitHub Actions 工作流通过 `actionlint` v1.7.7 检查。官方 `@actions/expressions` 求值覆盖 6 组事件与引用组合：只有推送 `v*` 标签会发布 Release，手动触发只上传构建产物。

## 固定测试集

`tests/fixtures/bot_realism/` 存放 12 张合成测试图和校验清单 `manifest.json`：

- `01-light-chat.png`：浅色聊天界面
- `02-dark-chat.png`：深色聊天界面
- `03-code.png`：代码编辑器
- `04-web-small-text.png`：小字号网页排版
- `05-avatar-emoji.png`：头像与彩色表情
- `06-photo.jpg`：合成风景照片
- `07-thin-lines.png`：1 像素细线网格
- `08-dashboard-odd.png`：奇数宽高监控面板
- `09-portrait-odd.png`：奇数宽高竖屏页面
- `10-grayscale.png`：灰度界面图
- `11-long.png`：超长纵向截图
- `12-ui-chrome.png`：浏览器标签栏与控件

校验命令：

```bash
cargo run --release --locked --features dev-tools --bin screenphoto-dev -- verify-corpus
```

`verify-corpus` 逐张核对 SHA-256 摘要、宽高和色彩模式，并核对 12 张图合计 69 个关键采样框。每个采样框内的像素灰度跨度必须达到至少 80 级，防止测试图退化为纯色块。

## 基准测试

测量配置：

- 构建配置：`release`，`rustc 1.98.1`
- 运行环境：WSL2 `x86_64`，Linux 6.18，20 逻辑 CPU
- 测试样本：12 张固定测试图，每张预热后同进程连续测量 10 次
- 固定参数：`seed = 20261107`，取自 `devtools::SAMPLE_SEED`，长边上限 `1920`
- 计时范围：`render_bytes` 完整流程，含解码、拍屏渲染和 JPEG 编码

| 输入 | P50 (ms) | P95 (ms) | 最慢 (ms) |
|---|---:|---:|---:|
| 01-light-chat.png | 465.8 | 473.2 | 473.3 |
| 02-dark-chat.png | 469.4 | 483.7 | 488.0 |
| 03-code.png | 310.9 | 330.6 | 334.8 |
| 04-web-small-text.png | 390.3 | 456.0 | 478.4 |
| 05-avatar-emoji.png | 163.4 | 166.1 | 167.4 |
| 06-photo.jpg | 325.7 | 330.4 | 331.9 |
| 07-thin-lines.png | 187.6 | 201.7 | 203.7 |
| 08-dashboard-odd.png | 211.0 | 212.8 | 212.9 |
| 09-portrait-odd.png | 214.8 | 364.1 | 483.1 |
| 10-grayscale.png | 302.9 | 313.4 | 316.1 |
| 11-long.png | 137.6 | 142.8 | 146.6 |
| 12-ui-chrome.png | 175.0 | 177.9 | 179.0 |
| 全部样本 | 300.6 | 471.1 | 488.0 |

完整原始数据保存在 [benchmark.json](benchmark.json)。改动渲染核心后重跑基准测试：

```bash
cargo build --release --locked --features dev-tools --bins
./target/release/screenphoto-dev benchmark --json docs/benchmark.json
```

## 样图

`samples/` 目录下的展示图由 `./target/release/screenphoto-dev gallery` 基于固定种子生成，源图 `test.jpg`、`social.jpg`、`poster.jpg` 随仓库保存：

- [彩图拍屏](../samples/photo.jpg) · [社交页面拍屏](../samples/social-photo.jpg) · [海报拍屏](../samples/poster-photo.jpg)
- [彩图缩略图](../samples/thumbnail.jpg) · [随机种子对比](../samples/seeds.jpg) · [实拍对照](../samples/reference.jpg) · [12 类输入总览](../samples/corpus.jpg)

## 打包验证

下表记录一次本地构建的平台包体积。实际发布体积随编译器和代码变化，以对应 Release 的资产为准，`SHA256SUMS` 用于校验完整性：

| 平台 | 可执行文件字节 | ZIP 字节 |
|---|---:|---:|
| Linux x86_64 musl | 1,615,512 | 762,628 |
| Windows x86_64 MSVC | 1,544,192 | 697,912 |

跨平台构建与安装器实测结果：

- Linux `x86_64-unknown-linux-musl`：完成静态编译、打包和解压运行验证，ELF 无外部动态库依赖。
- Windows `x86_64-pc-windows-msvc`：在原生 Windows 环境完成 MSVC 静态 CRT 编译、PE 导入表检查、解压后 `shot` 和真实桌面 `capture` 验证。
- `skills@1.8.1`，Node.js 24.21.0：本地技能目录可直接安装并保留 Linux `0755` 权限。通过 HTTP 下载 ZIP 安装时，Windows 包可直接运行，Linux 包因 ZIP 解压不保留权限位而变为 `0644`，执行 `chmod +x` 后运行正常。直接对源码目录运行 `skills add` 只复制仓库文件，不生成 `bin/screenphoto`。
