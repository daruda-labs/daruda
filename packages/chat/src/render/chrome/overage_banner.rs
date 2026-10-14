//! The banner a pane shows after stopping a turn to avoid extra-usage billing.

use chrono::{TimeZone, Utc};
use gpui::{Context, IntoElement, SharedString, div, prelude::*, px};

use crate::stamp;
use crate::strings as s;
use crate::view::{AgentChatView, OverageNotice};
use daruda_acp::RateLimitWindow;
use daruda_ui::theme;

/// The reset moment in the local zone. A 5-hour window resets within the
/// day, so the time alone reads; anything longer needs its date.
fn reset_time(notice: &OverageNotice) -> Option<String> {
    let when = Utc.timestamp_opt(notice.resets_at?, 0).single()?;
    Some(match notice.window {
        Some(RateLimitWindow::FiveHour) => stamp::local_time(when),
        _ => stamp::local_date_time(when),
    })
}

fn banner_text(window: &str, time: Option<&str>) -> String {
    match time {
        Some(time) => s::agent_chat::overage_stopped_resets(window, time),
        None => s::agent_chat::overage_stopped(window),
    }
}

pub(in crate::render) fn overage_banner(
    notice: Option<&OverageNotice>,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> Option<impl IntoElement + use<>> {
    let notice = notice?;
    let window = notice.window_label();
    let text = SharedString::from(banner_text(&window, reset_time(notice).as_deref()));
    Some(
        div()
            .flex_none()
            .w_full()
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y))
            .bg(t.banner_warning_bg)
            .text_color(t.banner_warning_text)
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(text),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_windows_get_a_label_and_unknown_ones_stay_verbatim() {
        let label = |window| {
            OverageNotice {
                window,
                resets_at: None,
            }
            .window_label()
        };
        assert_eq!(label(Some(RateLimitWindow::FiveHour)), "5-hour");
        assert_eq!(label(Some(RateLimitWindow::SevenDay)), "7-day");
        let unknown = RateLimitWindow::Other("seven_day_opus".to_owned());
        assert_eq!(label(Some(unknown)), "seven_day_opus");
        assert_eq!(label(None), "Usage");
    }

    #[test]
    fn the_reset_part_appears_only_with_a_reset_time() {
        assert_eq!(
            banner_text("5-hour", Some("3:00 PM")),
            "5-hour limit reached — stopped to avoid extra usage · resets 3:00 PM"
        );
        assert_eq!(
            banner_text("5-hour", None),
            "5-hour limit reached — stopped to avoid extra usage"
        );
    }

    #[test]
    fn no_reset_time_without_a_timestamp() {
        let notice = OverageNotice {
            window: Some(RateLimitWindow::FiveHour),
            resets_at: None,
        };
        assert_eq!(reset_time(&notice), None);
    }
}
