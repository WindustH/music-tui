use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(version, about = "Terminal music player backed by MPD")]
pub struct Cli {
  #[command(subcommand)]
  pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
  /// Open a folder or audio file and control how it joins the queue.
  Open(OpenArgs),
}

#[derive(Debug, Args)]
pub struct OpenArgs {
  /// Folder or audio file to open.
  pub path: PathBuf,

  /// Include audio files from subfolders recursively.
  #[arg(short, long)]
  pub recursive: bool,

  /// How the opened file joins the queue. For folders, `append` adds the
  /// songs to the current queue while every other mode replaces it.
  #[arg(short, long, default_value_t = OpenMode::Interrupt, value_enum)]
  pub mode: OpenMode,

  /// Queue songs without starting playback.
  #[arg(long)]
  pub no_play: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OpenMode {
  /// Append the file (or, for folders, every song inside) to the end of the
  /// queue.
  Append,
  /// Insert the file right after the currently playing song.
  Next,
  /// Play the file immediately; when it finishes, restore the previous queue
  /// and playback state.
  Interrupt,
  /// Play the file immediately and replace the queue with its folder.
  Folder,
}
