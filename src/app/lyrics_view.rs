//! Lyrics pane interaction: the manual cursor, viewport scrolling, follow
//! mode, and seeking to synced lines. Hovered lyrics panes have no
//! playback state, so they only scroll.

use super::*;

impl App {
  pub(crate) fn playing_lyrics(&self) -> Option<&Lyrics> {
    self.playing.as_ref()?.lyrics.as_ref()
  }

  pub(crate) fn active_lyrics_index(&self) -> Option<usize> {
    self
      .playing_lyrics()?
      .active_index(Duration::from_secs_f64(self.elapsed()))
  }

  /// Whether the active tab's main lyrics pane reads a hovered song.
  fn hover_lyrics_active(&self) -> bool {
    self.main_pane() == PaneKind::Lyrics && self.main_pane_source() != PaneSource::Playing
  }

  /// Keyboard scrolling in the lyrics pane: hovered lyrics scroll their
  /// viewport, the playing song's lyrics move the manual cursor.
  pub(crate) fn lyrics_key_scroll(&mut self, delta: i32) -> bool {
    if self.hover_lyrics_active() {
      self.scroll_hover_lyrics(delta);
    } else {
      self.move_lyrics_cursor(delta);
    }
    true
  }

  /// `enter`: seek to the highlighted (cursor or active) lyric line and
  /// resume auto-follow.
  pub(crate) fn lyrics_jump(&mut self) -> bool {
    if self.hover_lyrics_active() {
      self.set_message("hovered lyrics: song is not playing");
      return true;
    }
    let Some(index) = self.lyrics_cursor.or_else(|| self.active_lyrics_index()) else {
      return false;
    };
    self.lyrics_seek_to(index)
  }

  pub(crate) fn toggle_lyrics_follow(&mut self) -> bool {
    if self.hover_lyrics_active() {
      self.set_message("hovered lyrics: song is not playing");
      return true;
    }
    self.lyrics_follow = !self.lyrics_follow;
    if self.lyrics_follow {
      self.lyrics_cursor = None;
    }
    self.set_message(if self.lyrics_follow {
      "lyrics: following playback"
    } else {
      "lyrics: manual scroll"
    });
    true
  }

  fn move_lyrics_cursor(&mut self, delta: i32) {
    self.lyrics_follow = false;
    let cursor = self
      .lyrics_cursor
      .or_else(|| self.active_lyrics_index())
      .unwrap_or(0);
    self.lyrics_cursor = self
      .playing_lyrics()
      .and_then(|lyrics| lyrics.move_item_index(cursor, delta));
  }

  /// Max inner height of the lyrics panes in the current tab (viewport
  /// height for scroll clamping), recorded at draw time.
  fn lyrics_view_height(&self) -> usize {
    self
      .hit
      .lyrics_panes
      .iter()
      .map(|(area, _)| usize::from(area.height))
      .max()
      .unwrap_or(0)
      .max(1)
  }

  /// Scroll the hovered song's lyrics (plain list — no playback state):
  /// whichever hover view the visible hovered lyrics pane shows.
  fn scroll_hover_lyrics(&mut self, delta: i32) -> bool {
    let source = self
      .hit
      .lyrics_panes
      .iter()
      .map(|(_, source)| *source)
      .find(|source| *source != PaneSource::Playing)
      .unwrap_or(PaneSource::QueueHovered);
    let height = self.lyrics_view_height();
    let Some(hover) = self.song_view_mut(source) else {
      return false;
    };
    let line_count = hover.lyrics.as_ref().map(Lyrics::line_count).unwrap_or(0);
    if line_count == 0 {
      return false;
    }
    let max_scroll = line_count.saturating_sub(height);
    hover.lyrics_scroll = hover
      .lyrics_scroll
      .saturating_add_signed(delta as isize)
      .min(max_scroll);
    true
  }

  /// Scroll the playing lyrics viewport (wheel): the offset moves, the
  /// pointer passively follows and is clamped back inside the new
  /// viewport — same semantics as the queue view.
  fn scroll_lyrics_viewport(&mut self, delta: i32) -> bool {
    self.lyrics_follow = false;
    let line_count = self.playing_lyrics().map(Lyrics::line_count).unwrap_or(0);
    if line_count == 0 {
      return false;
    }
    let height = self.lyrics_view_height();
    let pointer = self
      .lyrics_cursor
      .unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
    let Some(playing) = self.playing.as_mut() else {
      return false;
    };
    let max_scroll = line_count.saturating_sub(height);
    let next = playing
      .lyrics_scroll
      .saturating_add_signed(delta as isize)
      .min(max_scroll);
    playing.lyrics_scroll = next;
    let last_visible = (next + height)
      .saturating_sub(1)
      .min(line_count.saturating_sub(1));
    self.lyrics_cursor = Some(pointer.clamp(next, last_visible));
    true
  }

  /// Lyrics pane under the pointer, with the data source it shows.
  pub(crate) fn lyrics_pane_at(&self, mouse: MouseEvent) -> Option<(Rect, PaneSource)> {
    self
      .hit
      .lyrics_panes
      .iter()
      .copied()
      .find(|(area, _)| viewport::contains(*area, mouse))
  }

  /// Click on a lyrics pane: synced lines seek to their timestamp;
  /// hovered panes have nothing to seek.
  pub(crate) fn click_lyrics(&mut self, area: Rect, source: PaneSource, mouse: MouseEvent) {
    if source != PaneSource::Playing {
      self.set_message("hovered lyrics: song is not playing");
      return;
    }
    let scroll = self
      .playing
      .as_ref()
      .map(|playing| playing.lyrics_scroll)
      .unwrap_or(0);
    let _ = self.lyrics_seek_to(usize::from(mouse.row - area.y) + scroll);
  }

  /// Wheel on a lyrics pane: scroll the pane's own source.
  pub(crate) fn scroll_lyrics_wheel(&mut self, source: PaneSource, delta: i32) -> bool {
    if source == PaneSource::Playing {
      self.scroll_lyrics_viewport(delta)
    } else {
      self.scroll_hover_lyrics(delta)
    }
  }

  /// Seek to a synced lyric line and return whether anything happened.
  pub(crate) fn lyrics_seek_to(&mut self, index: usize) -> bool {
    let Some(Lyrics::Synced(lines)) = self.playing_lyrics() else {
      return false;
    };
    let Some(time_secs) = lines.get(index).map(|line| line.time_secs) else {
      return false;
    };
    self.mpdc(MpdCommand::SeekCurrent(time_secs.max(0.0)));
    self.lyrics_follow = true;
    self.lyrics_cursor = None;
    self.set_message(format!("seek to {}", format_time(time_secs)));
    true
  }
}
