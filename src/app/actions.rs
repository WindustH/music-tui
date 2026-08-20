//! Keybinding action dispatch (`run_action`).

use super::*;

impl App {
  pub(crate) fn run_action(&mut self, action: &str) -> bool {
    match action {
      "quit" => {
        // With a secondary view open, `q` leaves that level instead of the
        // whole app (the global binding wins in every pane, so route here).
        if self.detail.is_some() {
          self.close_detail();
          true
        } else {
          self.quit = true;
          true
        }
      }
      "help" => {
        self.help_scroll = 0;
        self.show_help = true;
        true
      }
      "command" => {
        self.prompt = Some(Prompt::command(""));
        self.command_state.reset_prompt_state();
        self.refresh_prompt_completion();
        true
      }
      "tab_next" => self.cycle_tab(1),
      "tab_previous" => self.cycle_tab(-1),
      "back" => {
        if self.detail.is_some() {
          self.close_detail();
          true
        } else if self.main_pane() == PaneKind::Queue && self.queue_filter.is_some() {
          self.clear_queue_filter();
          true
        } else if self.main_pane() == PaneKind::Library && self.library_filter.is_some() {
          self.clear_library_filter();
          true
        } else {
          self.goto_tab(0)
        }
      }
      "queue_filter" => {
        let current = self.queue_filter.clone().unwrap_or_default();
        self.filter_target = FilterTarget::Queue;
        self.prompt = Some(Prompt::text("/", current));
        true
      }
      "library_filter" => {
        let current = self.library_filter.clone().unwrap_or_default();
        self.filter_target = FilterTarget::Library;
        self.prompt = Some(Prompt::text("/", current));
        true
      }
      "queue_up" => self.move_selection(-1),
      "queue_down" => self.move_selection(1),
      "queue_page_up" => self.queue_page(-1),
      "queue_page_down" => self.queue_page(1),
      "library_up" => self.move_library_selection(-1),
      "library_down" => self.move_library_selection(1),
      "library_page_up" => self.library_page(-1),
      "library_page_down" => self.library_page(1),
      "library_top" => {
        if self.library_visible_len() > 0 {
          self.select_library_row(0);
        }
        true
      }
      "library_end" => {
        let len = self.library_visible_len();
        if len > 0 {
          self.select_library_row(len - 1);
        }
        true
      }
      "library_play" => self.library_play_selected(),
      "library_append" => self.library_append_selected(),
      "library_detail" => self.open_library_detail(),
      "library_rescan" => self.library_rescan(),
      "queue_top" => {
        if self.visible_len() > 0 {
          self.queue_state.select(Some(0));
        }
        true
      }
      "queue_end" => {
        let len = self.visible_len();
        if len > 0 {
          self.queue_state.select(Some(len - 1));
        }
        true
      }
      "toggle_follow_current" => {
        self.follow_current = !self.follow_current;
        self.set_message(if self.follow_current {
          "following current song"
        } else {
          "selection unlocked from current song"
        });
        true
      }
      "queue_play" => {
        if let Some(position) = self
          .queue_state
          .selected()
          .and_then(|row| self.filtered_position(row))
        {
          self.mpdc(MpdCommand::PlayPosition(position as u32));
        }
        true
      }
      "play_pause" => {
        self.mpdc(MpdCommand::PlayPauseToggle);
        true
      }
      "next" => {
        self.mpdc(MpdCommand::Next);
        true
      }
      "previous" => {
        self.mpdc(MpdCommand::Previous);
        true
      }
      "stop" => {
        self.mpdc(MpdCommand::Stop);
        true
      }
      "queue_delete" => {
        if let Some(position) = self
          .queue_state
          .selected()
          .and_then(|row| self.filtered_position(row))
          && position < self.queue.len()
        {
          let title = song_title(&self.queue[position].song)
            .map(str::to_string)
            .unwrap_or_else(|| self.queue[position].song.url.clone());
          self.mpdc(MpdCommand::DeleteAt(position));
          self.set_message(format!("deleted: {title}"));
        }
        true
      }
      "queue_clear" => {
        self.mpdc(MpdCommand::ClearQueue);
        self.set_message("queue cleared");
        true
      }
      "queue_shuffle" => {
        self.mpdc(MpdCommand::Shuffle);
        self.set_message("queue shuffled");
        true
      }
      "queue_dedup" => {
        self.queue_dedup = !self.queue_dedup;
        self.mpd.set_queue_dedup(self.queue_dedup);
        self.recompute_queue_filter();
        self.clamp_queue_selection();
        if self.queue_dedup {
          // Enforce immediately: turning dedup on is itself a change the
          // current queue should comply with.
          self.enforce_queue_dedup();
        }
        self.set_message(if self.queue_dedup {
          "queue dedup on"
        } else {
          "queue dedup off"
        });
        true
      }
      "volume_up" => {
        self.mpdc(MpdCommand::NudgeVolume(5));
        true
      }
      "volume_down" => {
        self.mpdc(MpdCommand::NudgeVolume(-5));
        true
      }
      "volume_mute" => {
        let muted = self.status.as_ref().is_some_and(|status| status.volume == 0);
        self.mpdc(if muted { MpdCommand::SetVolume(50) } else { MpdCommand::SetVolume(0) });
        true
      }
      "seek_forward" => {
        self.mpdc(MpdCommand::NudgeSeek(5));
        true
      }
      "seek_back" => {
        self.mpdc(MpdCommand::NudgeSeek(-5));
        true
      }
      "seek_forward_long" => {
        self.mpdc(MpdCommand::NudgeSeek(30));
        true
      }
      "seek_back_long" => {
        self.mpdc(MpdCommand::NudgeSeek(-30));
        true
      }
      "toggle_repeat" => {
        self.mpdc(MpdCommand::SetRepeat(self.toggle_flag("repeat")));
        true
      }
      "toggle_random" => {
        self.mpdc(MpdCommand::SetRandom(self.toggle_flag("random")));
        true
      }
      "cycle_single" => {
        let next = match self.status.as_ref().map(|status| status.single) {
          Some(SingleMode::Disabled) => SingleMode::Enabled,
          Some(SingleMode::Enabled) => SingleMode::Oneshot,
          _ => SingleMode::Disabled,
        };
        self.mpdc(MpdCommand::SetSingle(next));
        true
      }
      "toggle_consume" => {
        self.mpdc(MpdCommand::SetConsume(self.toggle_flag("consume")));
        true
      }
      "scroll_up" => {
        self.scroll_metadata_by(-1);
        true
      }
      "scroll_down" => {
        self.scroll_metadata_by(1);
        true
      }
      "page_up" => {
        self.scroll_metadata_by(-10);
        true
      }
      "page_down" => {
        self.scroll_metadata_by(10);
        true
      }
      "edit_metadata" => {
        self.request_metadata_editor();
        true
      }
      "lyrics_up" => {
        if self.hover_lyrics_active() {
          self.scroll_hover_lyrics(-1);
        } else {
          self.lyrics_follow = false;
          let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
          self.lyrics_cursor = Some(cursor.saturating_sub(1));
        }
        true
      }
      "lyrics_down" => {
        if self.hover_lyrics_active() {
          self.scroll_hover_lyrics(1);
        } else {
          self.lyrics_follow = false;
          let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
          let limit = self.lyrics.as_ref().map(Lyrics::line_count).unwrap_or(1).saturating_sub(1);
          self.lyrics_cursor = Some((cursor + 1).min(limit));
        }
        true
      }
      "lyrics_page_up" => {
        if self.hover_lyrics_active() {
          self.scroll_hover_lyrics(-10);
        } else {
          self.lyrics_follow = false;
          let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
          self.lyrics_cursor = Some(cursor.saturating_sub(10));
        }
        true
      }
      "lyrics_page_down" => {
        if self.hover_lyrics_active() {
          self.scroll_hover_lyrics(10);
        } else {
          self.lyrics_follow = false;
          let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
          let limit = self.lyrics.as_ref().map(Lyrics::line_count).unwrap_or(1).saturating_sub(1);
          self.lyrics_cursor = Some((cursor + 10).min(limit));
        }
        true
      }
      "lyrics_jump" => {
        if self.hover_lyrics_active() {
          self.set_message("hovered lyrics: song is not playing");
          return true;
        }
        // Enter: seek to the highlighted (cursor or active) lyric line and
        // resume auto-follow.
        let index = self
          .lyrics_cursor
          .or_else(|| self.active_lyrics_index());
        let Some(index) = index else { return false };
        self.lyrics_seek_to(index)
      }
      "lyrics_follow" => {
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
      "queue_detail" => self.open_detail(),
      "queue_goto_playing" => self.goto_playing(),
      "visualizer_reset" => {
        self.spectrum.fill(0);
        true
      }
      _ => false,
    }
  }

}
