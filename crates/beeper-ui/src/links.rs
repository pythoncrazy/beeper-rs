//! Desktop's link previews of a message (`LinkPreview` and `VideoEmbed` in `MessageContent.tsx`): a card with the
//! preview image, the title, the summary and the host, or a YouTube card with the video thumbnail.
use crate::style::{clamp, fill_box, ic, icon, lh, text_width, txt};
use crate::{anim, BeeperGuiApp, El, UiMessage, BUBBLE_PAD_X};
use beeper_core::models::MessageLink;
use iced::font::Weight;
use iced::widget::{column, container, hover, image, mouse_area, row, stack, svg, Space};
use iced::{border, Center, Color, ContentFit, Fill, Length, Padding};
use std::sync::LazyLock;

/// The title that a link shows: its own, or the host of its URL when it has none.
pub(crate) fn shown_title(l: &MessageLink) -> String {
    let host = l.url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(&l.url);
    if l.title.is_empty() {
        host.to_string()
    } else {
        l.title.clone()
    }
}

/// Desktop's `narrowThreshold`: an image narrower than this, or a square one, shows at the left of the text.
const NARROW: u32 = 200;
/// `--chat-view-bubble-radius`.
const RADIUS: f32 = 17.0;

/// `.lty-playbtn`: the gray play button, and the red one under the cursor.
static PLAY: LazyLock<[svg::Handle; 2]> = LazyLock::new(|| {
    ["#212121", "#f00"].map(|fill| {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 68 48"><path d="M66.52 7.74c-.78-2.93-2.49-5.41-5.42-6.19C55.79.13 34 0 34 0S12.21.13 6.9 1.55c-2.93.78-4.63 3.26-5.42 6.19C.06 13.05 0 24 0 24s.06 10.95 1.48 16.26c.78 2.93 2.49 5.41 5.42 6.19C12.21 47.87 34 48 34 48s21.79-.13 27.1-1.55c2.93-.78 4.64-3.26 5.42-6.19C67.94 34.95 68 24 68 24s-.06-10.95-1.48-16.26z" fill="{fill}"/><path d="M45 24 27 14v20" fill="#fff"/></svg>"##
        );
        svg::Handle::from_memory(svg.into_bytes())
    })
});

/// Desktop's `getEmbedInfo` of YouTube: the video id of a watch, short, live or `youtu.be` link.
fn youtube_id(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1.split('#').next()?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let id = match host.to_lowercase().as_str() {
        "youtu.be" | "www.youtu.be" => parts.next(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => match parts.next()? {
            "watch" => query.split('&').find_map(|kv| kv.strip_prefix("v=")),
            "shorts" | "live" => parts.next(),
            _ => None,
        },
        _ => None,
    };
    id.filter(|id| !id.is_empty())
}

impl BeeperGuiApp {
    /// The image of a link at `w` by `h`, cut to cover, or an empty space while it loads. The image of a link preview
    /// of message `mid` is Desktop's `EnhancedImg playable`: an animation plays with "Autoplay GIFs" (`autoplay`), and
    /// without it shows its first frame with the play button.
    fn link_image<'a>(&'a self, mid: Option<(&str, bool)>, url: Option<&str>, (w, h): (f32, f32), radius: border::Radius) -> El<'a> {
        let Some((url, Some(Ok(frames)))) = url.map(|u| (u, self.media_at(u))) else {
            return Space::new().width(w).height(h).into();
        };
        let make = move |f| image(f).width(w).height(h).content_fit(ContentFit::Cover).border_radius(radius);
        let Some((mid, autoplay)) = mid else { return anim::gif(frames, self.shows(), self.started, make) };
        let key = (mid.to_string(), url.to_string());
        let played = self.pane().played_links.get(&key).filter(|_| !autoplay);
        let (play, start) = match played {
            Some(&at) => (self.shows(), at),
            None => (autoplay && self.shows(), self.started),
        };
        let gif = anim::gif(frames, play, start, make);
        match !autoplay && frames.len() > 1 {
            true => crate::message::gif_toggle(gif, played.is_some(), crate::message::Msg::ToggleLinkGif(key.0, key.1)),
            false => gif,
        }
    }

    /// The preview card of a link, `max` wide at most, in the colors that `dim` gives for a selected message. Like the
    /// `<a>` of Desktop's `LinkPreview`, a click on the card opens the link.
    /// A right click on the card opens the menu of message `mid` with the link's items. `autoplay` is Desktop's
    /// "Autoplay GIFs".
    pub(crate) fn view_link<'a>(
        &'a self, (mid, autoplay): (&str, bool), link: &MessageLink, max: f32, dim: impl Fn(Color) -> Color,
    ) -> El<'a> {
        let p = &self.pal;
        let menu = UiMessage::Message(crate::message::Msg::LinkMenu(mid.to_string(), link.url.clone()));
        if let Some(id) = youtube_id(&link.url) {
            return self.view_youtube(link, id, max, dim, menu);
        }
        let host = link.url.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(&link.url).to_string();
        let size = link.img_size.as_ref().and_then(|s| s.width.zip(s.height));
        let square = size.is_some_and(|(w, h)| w == h);
        let narrow = link.img.is_some() && (square || size.is_some_and(|(w, _)| w < NARROW));
        let meta = row![ic(icon::LINK, 12.0).color(dim(p.weak)), txt(host.clone(), 11.0).color(dim(p.weak))].spacing(4).align_y(Center);
        let natural = [Some(link.title.as_str()), link.summary.as_deref(), Some(&host)];
        let natural = natural.into_iter().flatten().map(|t| text_width(t, 13.0, Weight::Semibold)).fold(0.0, f32::max);
        let summary =
            |w: f32| link.summary.as_deref().map(|s| clamp(s, w, 3, 13.0, Weight::Normal).line_height(lh(16.0)).color(dim(p.strong)));
        let card: El<'a> = if narrow {
            // `.narrow`: a 120 px high row, with the image at the left and the title on one line.
            let (w, _) = size.expect("a narrow image has a size");
            let iw = if square { 120.0 } else { (w as f32).clamp(60.0, 120.0) };
            let text = (natural + 2.0 * BUBBLE_PAD_X).min(max - iw);
            let title = clamp(&link.title, text - 2.0 * BUBBLE_PAD_X, 1, 13.0, Weight::Semibold);
            let body = column![title.line_height(lh(16.0)).color(dim(p.strong))]
                .push(summary(text - 2.0 * BUBBLE_PAD_X))
                .push(Space::new().height(Fill))
                .push(meta)
                .spacing(8)
                .padding([12.0, BUBBLE_PAD_X])
                .width(text);
            let radius = border::Radius::default().top_left(RADIUS).bottom_left(RADIUS);
            row![self.link_image(Some((mid, autoplay)), link.img.as_deref(), (iw, 120.0), radius), body].height(120).into()
        } else {
            // A known image size makes the card 300 px wide, and an unknown one up to 360 px.
            let w = match (link.img.is_some(), size) {
                (true, Some(_)) => 300.0,
                _ => (natural + 2.0 * BUBBLE_PAD_X).min(360.0),
            }
            .min(max);
            let tw = w - 2.0 * BUBBLE_PAD_X;
            let title = clamp(&link.title, tw, 3, 13.0, Weight::Semibold).line_height(lh(16.0));
            let radius = border::Radius::default().top_left(RADIUS).top_right(RADIUS);
            let image = link.img.as_ref().map(|u| self.link_image(Some((mid, autoplay)), Some(u), (w, 160.0), radius));
            let text = column![title.color(dim(p.strong))].push(summary(tw)).push(meta).spacing(8);
            column![].push(image).push(container(text).padding(Padding::new(BUBBLE_PAD_X).top(8.0).bottom(8.0))).spacing(8).width(w).into()
        };
        let card = container(card).style(fill_box(dim(p.link_card), RADIUS));
        mouse_area(card).on_press(UiMessage::OpenUrl(link.url.clone())).on_right_press(menu).into()
    }

    /// Desktop's `VideoEmbed` of a YouTube link: the platform, the title, and the `lite-youtube` thumbnail with its
    /// play button. Desktop's title links to the video and its play button starts the player. iced has no web view for
    /// the player, so a click on the card opens the video in the browser.
    fn view_youtube<'a>(&'a self, link: &MessageLink, id: &str, max: f32, dim: impl Fn(Color) -> Color, menu: UiMessage) -> El<'a> {
        let p = &self.pal;
        let w = (0.45 * self.pane().size.width).min(400.0).min(max);
        let title = (!link.title.is_empty()).then(|| {
            let t = clamp(&link.title, w - 2.0 * BUBBLE_PAD_X, 3, 13.0, Weight::Semibold);
            t.line_height(lh(16.0)).color(dim(p.strong))
        });
        let name = crate::style::semi("YouTube", 11.0).line_height(lh(14.0)).color(dim(p.sub));
        // Only the title is Desktop's link there.
        let title = title.map(|t| mouse_area(t).on_right_press(menu));
        let meta = column![name].push(title).spacing(8).padding([8.0, BUBBLE_PAD_X]);
        let thumb = format!("https://i.ytimg.com/vi_webp/{id}/hqdefault.webp");
        let radius = border::Radius::default().bottom_left(RADIUS).bottom_right(RADIUS);
        let h = (w * 9.0 / 16.0).min(350.0);
        let shot = container(self.link_image(None, Some(&thumb), (w, h), radius)).style(fill_box(Color::BLACK, 0.0));
        let play = |i: usize, alpha: f32| {
            let b = svg(PLAY[i].clone()).width(68).height(48).opacity(alpha);
            container(b).center_x(w).center_y(h)
        };
        let video = hover(stack![shot, play(0, 0.8)], play(1, 1.0));
        let body = column![meta, video].spacing(8).width(w);
        let card = container(body).style(fill_box(dim(p.link_card), RADIUS)).max_height(350).width(Length::Shrink);
        mouse_area(card).on_press(UiMessage::OpenUrl(link.url.clone())).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_links_like_desktop() {
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=abc&t=10"), Some("abc"));
        assert_eq!(youtube_id("https://youtu.be/xyz?t=3"), Some("xyz"));
        assert_eq!(youtube_id("https://m.youtube.com/shorts/s1"), Some("s1"));
        assert_eq!(youtube_id("https://YouTube.com/live/l1#x"), Some("l1"));
        assert_eq!(youtube_id("https://www.youtube.com/channel/c"), None);
        assert_eq!(youtube_id("https://example.com/watch?v=abc"), None);
    }
}
