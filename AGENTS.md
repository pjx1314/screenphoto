# screenphoto 工作规则

本仓库维护 `screenphoto` Rust 核心库、命令行程序和 Agent Skill 包，当前版本号定义在 `Cargo.toml`。所有入口共用同一个渲染核心。

## 开始前

1. 读 [README.md](README.md) 了解项目定位。
2. 修改渲染、尺寸或打包逻辑前，读 [docs/internals.md](docs/internals.md)。
3. 修改命令行参数或 JSON 输出前，读 [docs/cli.md](docs/cli.md)。

## 开发命令

开发环境需要 Rust 1.88 以上版本和 Cargo。

```bash
cargo build --release --locked --bin screenphoto
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo check --locked --no-default-features --lib
cargo test --locked --no-default-features --test native_api
cargo build --release --locked --features dev-tools --bins
```

启用 `dev-tools` 特性后可运行维护工具：

```bash
./target/release/screenphoto-dev verify-corpus
./target/release/screenphoto-dev gallery
./target/release/screenphoto-dev benchmark --json docs/benchmark.json
```

## 代码目录

- `src/lib.rs`：导出公开渲染函数、输入上限和错误类型。
- `src/limits.rs`：定义输入字节数、输入像素数和输出长边上限。
- `src/color.rs`：维护 sRGB 查表，计算亮度和色调映射。
- `src/calibration.rs`：存放内部视觉校准参数。
- `src/render.rs`：校验并解码图片，校正方向、合成白底后编码 JPEG。
- `src/pipeline.rs`：调度拍屏渲染阶段，分配独立随机流。
- `src/geometry.rs`：计算针孔取景、透视矩阵和径向畸变，执行受限裁切。
- `src/optics/`：处理离焦模糊与重采样，叠加屏幕网格和环境反光，最后加入传感器噪声并锐化。
- `src/frame.rs`、`src/rng.rs`：提供线性光浮点 RGB 缓冲和确定性随机数生成器。
- `src/cli/`：解析命令行参数，生成执行计划，处理安全写入和 JSON 报告。
- `src/capture.rs`：通过 Windows GDI 抓取桌面像素。
- `src/devtools/`、`src/bin/screenphoto-dev.rs`：校验测试集，生成样图、基准数据和发布包。
- `.github/workflows/`：定义持续集成和标签发布工作流。
- `tests/`：存放公共接口测试、命令行集成测试和 `bot_realism` 固定测试集。

## 固定约束

- 公开的可调渲染参数只有 `seed`，输出长边上限固定为 `1920`。
- `seed` 未指定时生成 31-bit 非负整数，作为元组第二项返回。显式传入 `seed` 时支持完整 `u64`。复现条件见 [docs/internals.md](docs/internals.md#复现规则)。
- 编码输入上限为 40 MiB，输入像素上限为 24,000,000，大图只缩小不放大，输出 JPEG 不得写入 EXIF。
- 输出路径不得覆盖原图，包括符号链接、硬链接及规范化后相同的路径。
- 输出文件必须先写同目录临时文件再原子替换目标。写入失败时不得残留半截文件，也不得覆盖已有目标文件。
- 技能只用于在发送屏幕截图前调用 `shot` 生成拍屏 JPEG，不得直接发送原图。普通照片不得走此流程。

## 改动检查

- 修改前必须先读目标模块和全部调用方，不得顺手重排或清理无关代码。
- 新增依赖前必须先说明用途。不得引入系统图像库和 OpenCV，也不得引入 GPU 依赖或异步运行时。
- 修改视觉参数或渲染代码后，必须运行 `verify-corpus` 和 `gallery` 并人工检查样图。
- 修改影响耗时的代码后，必须重跑 `benchmark` 并同步更新 `docs/benchmark.json` 和 `docs/testing.md`。
- 修改命令行、JSON 字段或输入限制时，必须同步更新 `README.md`、`SKILL.md` 和 `docs/` 下对应文档。修改打包结构或版本号时同样必须同步更新。
- 提交前必须确认 `cargo fmt --all -- --check`、`cargo test --locked --all-features` 和 `cargo clippy --locked --all-targets --all-features -- -D warnings` 全部通过。
