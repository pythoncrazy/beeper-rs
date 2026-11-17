//! The avatars, media, quotes and media resolves that tasks load for the views.

use super::*;

#[derive(Clone)]
pub enum Msg {
    Avatar(String, Result<image::Handle, String>),
    Media(String, Result<Frames, String>),
    /// A contact card of the timeline, read from its `.vcf` file.
    VCard(String, Result<crate::vcard::VCard, String>),
    /// A PDF of the carousel, read from its file.
    Pdf(String, Result<crate::pdf::Doc, String>),
    Tile(String, image::Handle),
    /// A quoted message that the timeline did not have, or why it could not load.
    Quote(String, Result<Box<Message>, String>),
    /// Desktop's `mediaDownloadStore.download`: the interactive download of a media URL, and its end.
    Fetch(String),
    Fetched(String, Result<(), String>),
    ResolveMedia(String, String, String),
    MediaResolved(String, String, String, Result<(), String>),
    /// A load of items of the Media window.
    MediaPage(window::Id, media_view::MediaPage),
}

impl BeeperGuiApp {
    pub(crate) fn loaded(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Avatar(url, Ok(h)) => {
                self.avatars.insert(url, Some(h));
            }
            Msg::Avatar(_, Err(_)) => {}
            Msg::Media(url, r) => {
                if let Some(e) = r.as_ref().err().filter(|e| beeper_core::matrix::reload_needed(e)) {
                    eprintln!("media needs a download that the user asks for: {e}");
                    self.needs_download.insert(url.strip_suffix(media_view::LOOP).unwrap_or(&url).to_string());
                }
                self.media.insert(url, Some(r));
            }
            Msg::VCard(url, r) => {
                self.vcards.insert(url, Some(r));
            }
            // A PDF that loads after the carousel closed stays out.
            Msg::Pdf(url, r) => {
                if let Some(doc) = self.pdfs.get_mut(&url) {
                    *doc = Some(r);
                }
            }
            Msg::Tile(key, h) => self.tiles.insert(key, Some(h)),
            Msg::Fetch(url) => {
                if !self.fetching.insert(url.clone()) {
                    return Task::none();
                }
                let fetch = beeper_core::matrix::media_interactive(self.store.read().matrix.clone(), url.clone());
                return Task::perform(fetch, move |r| UiMessage::Loaded(Msg::Fetched(url.clone(), r)));
            }
            // The next view loads the media again, from the cache that the download filled.
            Msg::Fetched(url, r) => {
                self.fetching.remove(&url);
                match r {
                    Ok(()) => {
                        self.needs_download.remove(&url);
                        self.media.remove(&url);
                        self.media.remove(&format!("{url}{}", media_view::LOOP));
                    }
                    Err(e) => {
                        eprintln!("interactive download failed: {e}");
                        let text =
                            "Could not download this file. Check that the app is running on your phone and try again.";
                        self.fail(Toast::new(tr(text).into()));
                    }
                }
            }
            Msg::ResolveMedia(account_id, event_id, raw) => {
                if self.resolving_media.get(&event_id) == Some(&true) {
                    return Task::none();
                }
                self.resolving_media.insert(event_id.clone(), true);
                let p = self.provision(&account_id);
                let (eid, again) = (event_id.clone(), (account_id, raw.clone()));
                // Like Desktop, a resolve whose edit does not arrive in 30 s fails.
                let resolve = async move {
                    beeper_core::matrix::resolve_media(p, eid, raw).await?;
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    Ok(())
                };
                return Task::perform(resolve, move |r| {
                    UiMessage::Loaded(Msg::MediaResolved(again.0, event_id, again.1, r))
                });
            }
            Msg::MediaResolved(account_id, event_id, raw, r) => {
                self.resolving_media.insert(event_id.clone(), false);
                if let Err(e) = r {
                    eprintln!("resolve media failed: {e}");
                    let again = UiMessage::Loaded(Msg::ResolveMedia(account_id, event_id, raw));
                    self.fail(Toast {
                        buttons: vec![(tr("Try again"), again)],
                        ..Toast::new(tr("Could not load this media.").into())
                    });
                }
            }
            Msg::Quote(id, r) => _ = self.quotes.insert(id, Some(r.map(|m| *m))),
            Msg::MediaPage(id, page) => return Task::batch([self.media_paged(id, page), self.settle()]),
        }
        self.settle()
    }
}
