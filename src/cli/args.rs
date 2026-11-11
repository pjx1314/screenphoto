//! Clap surface: `shot` and `capture` with their flags.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "screenphoto",
    version,
    about = "把屏幕截图处理成手机拍屏照片。",
    subcommand_required = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// 把截图渲染成拍屏照
    Shot(ShotArgs),
    /// 抓当前屏幕并渲染
    Capture(CaptureArgs),
}

#[derive(Args)]
pub(crate) struct ShotArgs {
    /// 输入截图
    pub(crate) source: PathBuf,
    #[command(flatten)]
    pub(crate) render: RenderArgs,
}

#[derive(Args)]
pub(crate) struct CaptureArgs {
    /// 显示器序号（0 起）；不给则抓整个虚拟桌面
    #[arg(long)]
    pub(crate) monitor: Option<u32>,
    #[command(flatten)]
    pub(crate) render: RenderArgs,
}

#[derive(Args)]
pub(crate) struct RenderArgs {
    /// 随机种子；不传则每次随机，传入则复现同一张
    #[arg(long)]
    pub(crate) seed: Option<u64>,
    /// 输出 JPEG（shot 默认 <源文件名去掉扩展名>-photo.jpg，capture 默认 capture-photo.jpg）
    #[arg(short = 'o', long)]
    pub(crate) out: Option<PathBuf>,
    /// 只输出一行 JSON，供 agent 解析
    #[arg(long)]
    pub(crate) json: bool,
}
