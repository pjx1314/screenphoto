# 接口参考

`screenphoto` 提供 `shot` 和 `capture` 两个子命令，同时导出 `render_bytes` 和 `render_rgb` 两个 Rust 函数。仓库维护命令由 `screenphoto-dev` 提供。

## 命令概览

- `screenphoto shot <SOURCE> [OPTIONS]`：把本地 PNG 或 JPEG 渲染成手机拍屏 JPEG。
- `screenphoto capture [OPTIONS]`：在 Windows 上抓取屏幕像素并渲染成拍屏 JPEG。
- `screenphoto --version`：输出当前版本号。

## shot

```bash
screenphoto shot "截图.png" --seed 123 -o "out/photo.jpg" --json
```

选项说明：

- `--seed <N>`：指定 `u64` 随机种子，范围为 `0..=18446744073709551615`。省略时从系统随机源生成 31-bit 非负整数。
- `-o, --out <PATH>`：指定输出路径。省略时写入当前目录的 `<源文件名去掉扩展名>-photo.jpg`，父目录不存在时自动创建。Unix 下保留源文件名的原始字节，不同的非 UTF-8 文件名不会因替换字符生成同名输出。
- `--json`：在标准输出打印单行 JSON 报告。

注意：输出路径不得指向原图。符号链接、硬链接及规范化后指向同一位置的路径都会被拒绝。

### 成功输出

传入 `--json` 时，标准输出打印一行 JSON：

```json
{"ok":true,"input":"截图.png","output":"out/photo.jpg","width":1000,"height":1400,"bytes":250000,"seed":123}
```

`width` 和 `height` 读取自生成的 JPEG 文件头，`bytes` 为输出文件字节数。路径含有非 UTF-8 字节时，JSON 字符串显示替换字符。需要按 `output` 字段读取文件的调用方，宜通过 `-o` 传入合法的 UTF-8 路径。

### 退出码

失败时返回非 `0` 退出码。除参数解析错误和标准输出写入失败外，传入 `--json` 都会在标准输出打印错误 JSON：

```json
{"ok":false,"error":"找不到文件：missing.png","input":"missing.png"}
```

- 命令行参数错误，返回退出码 `2`：缺少必填参数、包含未知选项，或者参数类型错误、传入负数 `seed`。`clap` 直接把错误信息写到标准错误，不输出 JSON。
- 输入检查失败，返回退出码 `2`：输入文件不存在或无法读取。文件超限、图片损坏或输出路径指向原图同样返回 `2`。
- 处理或环境错误，返回退出码 `1`：随机源故障、JPEG 编码失败或输出缺少尺寸。写入目标文件失败或桌面抓屏失败同样返回 `1`。
- 报告输出失败，返回退出码 `1`：写入标准输出失败，诊断信息写到标准错误。此时 JPEG 文件可能已经写入磁盘，调用方以退出码为准。

## capture

```powershell
screenphoto capture --monitor 0 --seed 123 -o "out/capture.jpg" --json
```

`--monitor <N>` 从 `0` 开始编号。省略 `--monitor` 时抓取整个 Windows 虚拟桌面，省略 `-o` 时默认写入当前目录的 `capture-photo.jpg`。`--seed`、`-o` 和 `--json` 的行为与 `shot` 相同。非 Windows 平台运行 `capture` 返回退出码 `1`。

## 转换规格

`shot` 和 `render_bytes` 接收编码后的图片字节。`capture` 和 `render_rgb` 直接接收内存中的 RGB8 像素。四个入口共用同一套渲染规格：

- 编码输入支持 PNG 和 JPEG，文件大小范围为 `1 B..=40 MiB`，总像素上限为 `24,000,000`。
- 编码输入自动按 EXIF 方向旋转，灰度图转为 RGB，透明通道合成到纯白背景。
- 输出固定为质量 `92`、色度采样 `4:2:0` 的 JPEG，不写入 EXIF 元数据。
- 输出长边上限为 `1920`，小图保持原尺寸不放大。复现条件见 [内部结构](internals.md#复现规则)。
- 取景最多裁切每条轴两端合计 10% 的范围，原图中央 90% 宽高区域始终可见。
- 命令行写文件时，先写入目标目录内的临时文件并刷盘，再原子替换目标文件。中途失败不会留下半截文件，也不会破坏已有文件。细节见 [内部结构](internals.md#文件读写)。

## Rust 库

在 `Cargo.toml` 中引入：

```toml
[dependencies]
screenphoto = { path = "/path/to/screenphoto", default-features = false }
```

调用示例：

```rust
use screenphoto::render_bytes;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read("input.png")?;
    let (jpeg, seed) = render_bytes(&data, Some(123))?;
    std::fs::write("photo.jpg", jpeg)?;
    println!("seed={seed}");
    Ok(())
}
```

公开签名：

```rust
pub fn render_bytes(data: &[u8], seed: Option<u64>) -> screenphoto::Result<(Vec<u8>, u64)>;
pub fn render_rgb(width: u32, height: u32, pixels: Vec<u8>, seed: Option<u64>) -> screenphoto::Result<(Vec<u8>, u64)>;
```

`seed` 传 `None` 时，函数生成 31-bit 非负随机整数并随结果返回。`render_rgb` 要求 `pixels.len() == width * height * 3`，供抓屏等已有像素缓冲的场景跳过 PNG 编解码。

错误类型 `screenphoto::Error` 区分 `InvalidInput` 和 `Processing`，可通过 `Error::is_input()` 判断。`screenphoto::limits` 模块导出 `MAX_INPUT_BYTES`、`MAX_INPUT_PIXELS`、`LONG_EDGE_CAP`、`LimitError` 和 `checked_pixels`。

## 维护工具

开启 `dev-tools` 特性后可编译并运行 `screenphoto-dev`，所有子命令均支持通过 `--root <DIR>` 指定仓库根目录：

```bash
cargo build --release --locked --features dev-tools --bins
./target/release/screenphoto-dev verify-corpus
./target/release/screenphoto-dev gallery
./target/release/screenphoto-dev benchmark --json docs/benchmark.json
./target/release/screenphoto-dev package --binary ./target/release/screenphoto --target x86_64-unknown-linux-gnu --out dist
```

- `verify-corpus`：校验 `tests/fixtures/bot_realism/` 下的 12 张固定测试图与清单摘要。
- `gallery`：重新生成 `samples/` 目录下的展示样图与对比图。
- `benchmark`：测量 12 张固定测试图在 `render_bytes` 下的完整耗时。
- `package`：组装平台技能目录，生成发布 ZIP 与 `SHA256SUMS`。
