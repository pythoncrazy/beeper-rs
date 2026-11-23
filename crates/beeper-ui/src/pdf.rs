//! Desktop's PDF viewer in the carousel (`PDFAttachmentCarousel`, react-pdf over pdf.js): one page of the file at a
//! time, with Previous Page, the page number and Next Page over it. hayro, a PDF rasterizer in pure Rust, takes the
//! place of pdf.js: a worker reads the file once, and another draws only the page that the carousel shows, as media.

use crate::i18n::tr;
use crate::style::rem;
use crate::style::{fill_box, text_width, txt, Bare};
use crate::{anim, loads, media_view, BeeperGuiApp, El, Frames, UiMessage};
use beeper_core::models::Attachment;
use hayro::hayro_syntax::Pdf;
use iced::font::Weight;
use iced::widget::{button, container, image, mouse_area, row, svg, Space};
use iced::{Center, Color, Fill, Padding, Pixels, Size, Task};
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

/// A read PDF, which the renders of its pages share.
pub(crate) type Doc = Arc<Pdf>;

/// The suffix of the media key of a page of a PDF, before the page number from 1.
const PAGE: &str = "#page=";

/// The long side of a page render at most, in pixels.
const MAX_SIDE: f32 = 4096.0;

/// The height of Desktop's `.pdf-carousel-controls`: 12 px above and below the 14 px text with its 1.4 line height.
const CONTROLS_H: f32 = 44.0;

/// The arrows of Desktop's Previous Page and Next Page, 6 × 9 in a 14 px box. Previous Page mirrors Next Page.
static ARROWS: LazyLock<[svg::Handle; 2]> = LazyLock::new(|| {
    ["M5 1L1.5 4.5L5 8", "M1 1L4.5 4.5L1 8"].map(|d| {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 6 9" fill="none"><path d="{d}" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
        );
        svg::Handle::from_memory(svg.into_bytes())
    })
});

/// Desktop's `pb`: an attachment with a PDF MIME type or a `.pdf` name.
pub(crate) fn is_pdf(a: &Attachment) -> bool {
    let lower = |s: &Option<String>| s.as_deref().unwrap_or_default().to_lowercase();
    lower(&a.mime_type).ends_with("/pdf") || lower(&a.file_name).ends_with(".pdf")
}

/// The media key of page `n` of the PDF at `url`.
fn page_key(url: &str, n: usize) -> String {
    format!("{url}{PAGE}{n}")
}

/// Reads a PDF, like pdf.js `getDocument`. A PDF without pages fails like react-pdf's page of it.
fn read(bytes: Vec<u8>) -> Result<Doc, String> {
    let pdf = Pdf::new(Arc::new(bytes)).map_err(|e| format!("Could not read the PDF: {e:?}"))?;
    match pdf.pages().is_empty() {
        true => Err("The PDF has no pages".into()),
        false => Ok(Arc::new(pdf)),
    }
}

/// Page `n` (from 1) of `doc` on white, like react-pdf's canvas, at twice its size in points so that it stays sharp on
/// a HiDPI screen and in a zoom. The page is opaque, so its premultiplied pixels are plain RGBA.
fn render(doc: &Pdf, n: usize) -> Result<Frames, String> {
    let page = n.checked_sub(1).and_then(|i| doc.pages().get(i)).ok_or("No such page")?;
    let (w, h) = page.render_dimensions();
    let scale = 2.0f32.min(MAX_SIDE / w.max(h));
    let bg_color = hayro::vello_cpu::color::palette::css::WHITE;
    let settings = hayro::PixmapSettings { x_scale: scale, y_scale: scale, bg_color };
    let px = hayro::render(page, &hayro::RenderCache::new(), &Default::default(), &Default::default(), &settings);
    let (w, h) = (u32::from(px.width()), u32::from(px.height()));
    let rgba = px.take_rgba8(hayro::vello_cpu::peniko::ImageAlphaType::AlphaPremultiplied);
    Ok(Frames::from([(image::Handle::from_rgba(w, h, rgba), 0)]))
}

/// The size of a page of `w` × `h` points in a carousel of `area`: its size in CSS pixels, like react-pdf's canvas at
/// scale 1, scaled down into the carousel and Desktop's `max-height: calc(100vh - 110px)`.
fn fit((w, h): (f32, f32), area: Size) -> Size {
    let k = ((area.width - 16.0) / w).min((area.height - 110.0) / h).min(1.0);
    Size::new(w * k, h * k)
}

impl BeeperGuiApp {
    /// The PDF at `url`. A view that shows a PDF that no task loads asks for it.
    fn pdf_at(&self, url: &str) -> Option<&Result<Doc, String>> {
        match self.pdfs.get(url) {
            Some(d) => d.as_ref(),
            None => {
                self.wants.ask(&self.wants.pdfs, url);
                None
            }
        }
    }

    /// Starts the loads of the PDFs that the carousel asked for and the renders of the pages that it asked for, off
    /// the UI thread. The carousel asks for a page once its PDF is read.
    pub(crate) fn load_pdfs(&mut self) -> Task<UiMessage> {
        let client = self.store.read().matrix.clone();
        let loadable = |u: &String| client.is_some() || u.starts_with("https://") || u.starts_with("data:");
        let (urls, rest): (BTreeSet<_>, _) =
            std::mem::take(&mut *self.wants.pdfs.borrow_mut()).into_iter().partition(loadable);
        *self.wants.pdfs.borrow_mut() = rest;
        let urls: Vec<String> = urls.into_iter().filter(|u| !self.pdfs.contains_key(u)).collect();
        self.pdfs.extend(urls.iter().map(|u| (u.clone(), None)));
        let (keys, rest): (BTreeSet<_>, _) =
            std::mem::take(&mut *self.wants.media.borrow_mut()).into_iter().partition(|k| k.contains(PAGE));
        *self.wants.media.borrow_mut() = rest;
        let page = |k: &String| {
            let (url, n) = k.rsplit_once(PAGE)?;
            Some((self.pdfs.get(url)?.as_ref()?.as_ref().ok()?.clone(), n.parse().ok()?))
        };
        let keys = keys.into_iter().filter(|k| !self.media.contains_key(k));
        let pages: Vec<(String, (Doc, usize))> = keys.filter_map(|k| Some((k.clone(), page(&k)?))).collect();
        self.media.extend(pages.iter().map(|(k, _)| (k.clone(), None)));
        let reads = urls.into_iter().map(|url| {
            let bytes = beeper_core::matrix::media(client.clone(), url.clone());
            let doc = async move {
                let bytes = bytes.await?;
                tokio::task::spawn_blocking(move || read(bytes)).await.expect("PDF read task")
            };
            Task::perform(doc, move |r| UiMessage::Loaded(loads::Msg::Pdf(url.clone(), r)))
        });
        let draws = pages.into_iter().map(|(key, (doc, n))| {
            let page =
                async move { tokio::task::spawn_blocking(move || render(&doc, n)).await.expect("PDF render task") };
            Task::perform(page, move |r| UiMessage::Loaded(loads::Msg::Media(key.clone(), r)))
        });
        Task::batch(reads.chain(draws))
    }

    /// Desktop's `PDFAttachmentCarousel` of the PDF at `url` on page `n`, in a carousel of `area`: the page, turned
    /// like an image, react-pdf's "Loading PDF…", or "Unable to load PDF…". The second element is its
    /// `.pdf-carousel-controls`, 40 px above the bottom of the first and 40 px left of its middle, which goes over the
    /// zoom of the carousel.
    pub(crate) fn view_pdf<'a>(
        &'a self,
        url: &str,
        n: usize,
        turn: (f32, f32, Instant),
        area: Size,
    ) -> (El<'a>, El<'a>) {
        let white = Color::WHITE;
        // react-pdf's messages, which Desktop does not translate, and the error in the 16 px type of the carousel.
        let note = |s: &'a str| -> (El<'a>, Size) {
            let size = Size::new(text_width(s, 16.0, Weight::Normal), 22.4);
            (txt(s, 16.0).line_height(Pixels(rem(22.4))).color(white).into(), size)
        };
        let (count, (shown, size)) = match self.pdf_at(url) {
            None => (None, note("Loading PDF…")),
            Some(Err(_)) => (None, note(tr("Unable to load PDF. Download the file to view."))),
            Some(Ok(doc)) => {
                let shown = match doc.pages().get(n - 1) {
                    // react-pdf's page error, for a page that Next Page reached before the PDF was read.
                    None => note("Failed to load the page."),
                    Some(page) => {
                        let size = fit(page.render_dimensions(), area);
                        // The page is white while it draws, like react-pdf's page under its canvas.
                        let page: El<'a> = match self.media_at(&page_key(url, n)) {
                            Some(Ok(frames)) => anim::turning(frames, false, self.started, turn, move |f| {
                                image(f).width(size.width).height(size.height)
                            }),
                            _ => container(Space::new())
                                .width(size.width)
                                .height(size.height)
                                .style(fill_box(white, 0.0))
                                .into(),
                        };
                        (page, size)
                    }
                };
                (Some(doc.pages().len()), shown)
            }
        };
        let arrow = |next: bool, to: Option<usize>| -> El<'a> {
            let glyph = svg(ARROWS[usize::from(next)].clone())
                .width(14)
                .height(14)
                .opacity(if to.is_some() { 1.0f32 } else { 0.5 })
                .style(move |_, _| svg::Style { color: Some(white) });
            button(glyph).bare(white).on_press_maybe(to.map(|n| UiMessage::Media(media_view::Msg::PdfPage(n)))).into()
        };
        let counter = format!("{n}/{}", count.map(|c| c.to_string()).unwrap_or_default());
        let line = row![
            arrow(false, (n > 1).then(|| n - 1)),
            Space::new().width(Fill),
            container(txt(counter, 14.0).line_height(Pixels(rem(19.6))).color(white))
                .padding(Padding::ZERO.bottom(2.0)),
            Space::new().width(Fill),
            arrow(true, (Some(n) != count).then_some(n + 1)),
        ]
        .align_y(Center);
        let controls = container(line).width(100).padding([0, 8]).center_y(CONTROLS_H);
        let controls =
            mouse_area(controls.style(fill_box(Color::BLACK.scale_alpha(0.5), 16.0))).on_press(UiMessage::Noop);
        // The middle of the carousel below its 50 px header, inside its 8 px margin.
        let (x, y) = (area.width / 2.0, 50.0 + 8.0 + (area.height - 66.0) / 2.0);
        let at = Padding::ZERO.left(x - 40.0).top(y + size.height / 2.0 - 40.0 - CONTROLS_H);
        (shown, container(controls).padding(at).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::clicks;
    use crate::fuzz::demo_app;
    use crate::message;
    use iced::Point;
    use iced_test::simulator::{simulator, Simulator};

    /// The demo chat, message and file of `beeper-v4-spec.pdf`, its second attachment.
    const CHAT: &str = "!tg_rust:beeper.com";
    const MID: &str = "msg_1003";
    const URL: &str = "mxc://beeper.com/att_pdf_1";

    /// A PDF of `pages` pages of 200 × 100 points, each with a black square in its top left quarter.
    fn sample(pages: usize) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect::<Vec<_>>().join(" ")
            ),
        ];
        let content = 3 + pages;
        objects.extend(
            (0..pages)
                .map(|_| format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents {content} 0 R >>")),
        );
        let draw = "0 0 0 rg 0 50 100 50 re f";
        objects.push(format!("<< /Length {} >>\nstream\n{draw}\nendstream", draw.len()));
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        pdf.extend(offsets.iter().flat_map(|o| format!("{o:010} 00000 n \n").into_bytes()));
        pdf.extend(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes(),
        );
        pdf
    }

    #[test]
    fn pdfs_by_mime_type_or_name_like_desktop() {
        let a = |mime: Option<&str>, name: Option<&str>| Attachment {
            mime_type: mime.map(Into::into),
            file_name: name.map(Into::into),
            ..Default::default()
        };
        assert!(is_pdf(&a(Some("application/PDF"), None)));
        assert!(is_pdf(&a(None, Some("Spec.PDF"))));
        assert!(is_pdf(&a(Some("application/octet-stream"), Some("spec.pdf"))));
        assert!(!is_pdf(&a(Some("text/plain"), Some("spec.pdf.txt"))));
        assert!(!is_pdf(&a(None, None)));
    }

    /// hayro reads the page count and draws one page at twice its size, on white.
    #[test]
    fn a_page_renders_at_twice_its_size_on_white() {
        let doc = read(sample(3)).expect("a valid PDF");
        assert_eq!(doc.pages().len(), 3);
        let frames = render(&doc, 2).expect("page 2");
        let image::Handle::Rgba { width, height, pixels, .. } = &frames[0].0 else { panic!("RGBA pixels") };
        assert_eq!((*width, *height), (400, 200), "twice the size in points");
        let px = |x: usize, y: usize| &pixels[(y * 400 + x) * 4..][..4];
        assert_eq!(px(50, 50), [0, 0, 0, 255], "the square is black");
        assert_eq!(px(300, 150), [255, 255, 255, 255], "the page is white");
        assert!(render(&doc, 4).is_err() && render(&doc, 0).is_err(), "no page out of the PDF");
    }

    #[test]
    fn a_file_that_is_not_a_pdf_does_not_read() {
        assert!(read(b"not a PDF".to_vec()).is_err());
        assert!(read(Vec::new()).is_err());
    }

    /// A page shows at its size in points, scaled down into the carousel below Desktop's 110 px.
    #[test]
    fn a_page_fits_in_the_carousel() {
        assert_eq!(fit((612.0, 792.0), Size::new(1400.0, 1000.0)), Size::new(612.0, 792.0));
        assert_eq!(fit((612.0, 792.0), Size::new(1000.0, 506.0)), Size::new(306.0, 396.0));
        assert_eq!(fit((2000.0, 100.0), Size::new(1016.0, 800.0)), Size::new(1000.0, 50.0));
    }

    fn opens(m: &UiMessage) -> bool {
        matches!(m, UiMessage::Media(media_view::Msg::OpenCarousel(m, 1, false)) if m == MID)
    }

    /// Desktop's `AttachmentFileRow` of a PDF opens it in the carousel, and the menu of a message with a PDF has
    /// "Preview in Carousel", which opens its first PDF.
    #[test]
    fn a_pdf_opens_in_the_carousel_like_desktop() {
        let mut app = demo_app("pdf-open");
        let _ = app.update(UiMessage::SelectChat(CHAT.into()));
        let mut sim = Simulator::with_size(Default::default(), Size::new(1200.0, 900.0), app.view_main());
        let _ = sim.click("beeper-v4-spec.pdf").expect("the PDF row");
        assert!(sim.into_messages().any(|m| opens(&m)), "a click on the row opens the carousel");
        let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(MID.into())));
        let msgs: Vec<UiMessage> = clicks(app.view_main(), "Preview in Carousel");
        assert!(matches!(&msgs[..], [UiMessage::Message(message::Msg::Menu(m))] if opens(m)), "the item opens the PDF");
        let image = {
            let st = app.store.read();
            st.messages.iter().find(|m| m.attachments.iter().any(|a| a.attachment_type == "img")).unwrap().id.clone()
        };
        let _ = app.update(UiMessage::Message(message::Msg::OpenMenu(image)));
        assert!(simulator(app.view_main()).find("Preview in Carousel").is_err(), "only a message with a PDF has it");
    }

    /// Desktop's `PDFAttachmentCarousel`: "Loading PDF…" while the file loads, then the first page and "1/3", where
    /// Previous Page is off. Next Page goes on until the last page, only the shown page draws, and a click on the
    /// controls keeps the carousel open. A PDF that does not load shows "Unable to load PDF…", and the close lets go of
    /// the PDF, so that the next open starts again on page 1.
    #[test]
    fn the_carousel_pages_through_a_pdf_like_desktop() {
        let mut app = demo_app("pdf-pages");
        let _ = app.update(UiMessage::SelectChat(CHAT.into()));
        app.pane.size = Size::new(1000.0, 720.0);
        let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(MID.into(), 1, false)));
        fn sim(app: &BeeperGuiApp) -> Simulator<'_, UiMessage> {
            Simulator::with_size(Default::default(), app.pane.size, app.view_main())
        }
        assert!(sim(&app).find("Loading PDF…").is_ok() && sim(&app).find("1/").is_ok(), "the PDF loads");
        assert!(app.wants.pdfs.borrow().contains(URL), "the carousel asks for the PDF");
        let doc = read(sample(3)).unwrap();
        app.pdfs.insert(URL.into(), Some(Ok(doc.clone())));
        assert!(sim(&app).find("1/3").is_ok(), "the page and the page count");
        let _ = app.update(UiMessage::LoadWanted);
        assert!(app.media.contains_key(&page_key(URL, 1)), "the shown page draws");
        assert!(!app.media.contains_key(&page_key(URL, 2)), "no other page draws");
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(page_key(URL, 1), render(&doc, 1))));
        // The 200 × 100 page shows in the middle of the carousel below its header, at x 400 to 600 and y 335 to 435,
        // and the 100 × 44 controls from x 460 and 40 px above its bottom, with Previous Page at x 468 and Next Page at
        // x 538. The test reads the pixels of the page, which the simulator does not find.
        let shot = crate::focus::tests::pixels(&mut sim(&app), &iced::Theme::Light, "pdf");
        let k = shot.width() / 1000;
        let px = |x: u32, y: u32| shot.get_pixel(x * k, y * k).0;
        assert_eq!(px(430, 360), [0, 0, 0, 255], "the square of the page");
        assert_eq!(px(580, 420), [255, 255, 255, 255], "the white of the page");
        assert!(px(500, 392)[..3].iter().all(|&c| (120..136).contains(&c)), "the half black controls over the page");
        assert_eq!(px(520, 345), [255, 255, 255, 255], "the page above the controls");
        let click = |app: &BeeperGuiApp, x: f32| -> Vec<UiMessage> {
            let mut sim = sim(app);
            sim.point_at(Point::new(x, 373.0));
            let _ = sim.simulate(iced_test::simulator::click());
            sim.into_messages().collect()
        };
        let pages = |msgs: &[UiMessage]| -> Vec<usize> {
            let page = |m: &UiMessage| match m {
                UiMessage::Media(media_view::Msg::PdfPage(n)) => Some(*n),
                _ => None,
            };
            msgs.iter().filter_map(page).collect()
        };
        let back = click(&app, 475.0);
        assert!(pages(&back).is_empty(), "Previous Page is off on the first page");
        let close = |m: &UiMessage| matches!(m, UiMessage::Media(media_view::Msg::CloseCarousel));
        assert!(!back.iter().any(close), "a click on the controls keeps the carousel");
        assert_eq!(pages(&click(&app, 545.0)), [2], "Next Page goes to the next page");
        let _ = app.update(UiMessage::Media(media_view::Msg::PdfPage(3)));
        assert!(sim(&app).find("3/3").is_ok());
        assert!(pages(&click(&app, 545.0)).is_empty(), "Next Page is off on the last page");
        assert_eq!(pages(&click(&app, 475.0)), [2], "Previous Page goes back");
        app.pdfs.insert(URL.into(), Some(Err("Could not read the PDF".into())));
        assert!(sim(&app).find("Unable to load PDF. Download the file to view.").is_ok(), "a PDF that does not load");
        let _ = app.update(UiMessage::Media(media_view::Msg::CloseCarousel));
        assert!(app.pdfs.is_empty(), "the close lets go of the PDF");
        let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(MID.into(), 1, false)));
        assert_eq!(app.pane.carousel.as_ref().unwrap().page, 1, "a new carousel starts on page 1");
    }
}
