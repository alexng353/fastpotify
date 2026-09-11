//! Navigation and asynchronous state for song-radio pages.

use super::*;

impl App {
    pub(super) fn cancel_radio_play(&mut self) {
        if self.pending_radio_play.take().is_some() {
            self.clear_play_pending();
        }
    }

    pub(super) fn start_radio(&mut self, uri: &str) {
        let Some(id) = util::uri_id(uri) else {
            return;
        };
        self.cancel_radio_play();
        self.queued_play = None;
        let page = self.radio_pages.entry(id.to_owned()).or_default();
        if matches!(page.station, Loadable::Failed(_)) {
            page.station = Loadable::NotLoaded;
        }
        self.load_radio(id);
        let generation = self.radio_pages[id].generation;
        self.pending_radio_play = Some((id.to_owned(), generation));
        self.set_play_pending(vec![uri.to_owned()]);
        self.queue_tab = QueueTab::Queue;
        if !matches!(self.page(), Page::Queue) && !self.show_queue_panel {
            self.show_queue_panel = true;
            self.show_lyrics_panel = false;
        }
        self.play_loaded_radio(id, generation);
    }

    fn play_loaded_radio(&mut self, id: &str, generation: u64) {
        if !self
            .pending_radio_play
            .as_ref()
            .is_some_and(|(pending_id, pending_generation)| {
                pending_id == id && *pending_generation == generation
            })
        {
            return;
        }
        let Some(station) = self.radio_pages.get(id).and_then(|page| page.station.get()) else {
            return;
        };
        let context = station.uri.clone();
        let uris: Vec<String> = station
            .tracks
            .iter()
            .map(|track| track.uri.clone())
            .collect();
        self.cancel_radio_play();
        let Some(first) = uris.first().cloned() else {
            self.toast("Spotify returned no songs for this radio. Try again.");
            return;
        };
        self.expect_track(first.clone());
        self.set_play_pending(vec![first]);
        self.note_recent_context(&context);
        self.local_list = Some(uris.clone());
        self.backend.player(PlayerCommand::Load(LoadSpec {
            uris,
            play: true,
            ..LoadSpec::default()
        }));
        self.optimistic_playing = Some((true, Instant::now()));
        self.assumed_context = Some(AssumedContext {
            uri: context,
            shuffle: None,
            at: Instant::now(),
        });
        self.refresh_queue(true);
    }

    pub(crate) fn radio_seed(&self, id: &str) -> Option<&Track> {
        self.radio_pages
            .get(id)
            .and_then(|page| {
                page.station
                    .get()
                    .map(|station| &station.seed)
                    .or(page.seed.as_ref())
            })
            .or_else(|| self.track_cache.get(id))
    }

    pub(super) fn open_radio(&mut self, track: Track) {
        let Some(id) = util::uri_id(&track.uri).map(str::to_owned) else {
            return;
        };
        self.radio_pages.entry(id.clone()).or_default().seed = Some(track);
        self.open(Page::Radio(id));
    }

    pub(super) fn load_radio(&mut self, id: &str) {
        if self
            .radio_pages
            .get(id)
            .is_some_and(|page| !page.station.needs_load())
        {
            return;
        }
        self.load_generation += 1;
        let page = self.radio_pages.entry(id.to_owned()).or_default();
        if page.seed.is_none() {
            page.seed = self.track_cache.get(id).cloned();
        }
        page.generation = self.load_generation;
        page.station = Loadable::Loading;
        if let Some((pending_id, generation)) = self.pending_radio_play.as_mut()
            && pending_id == id
        {
            *generation = page.generation;
        }
        self.backend.send(Command::Radio {
            id: id.to_owned(),
            generation: page.generation,
        });
    }

    pub(super) fn receive_radio(
        &mut self,
        id: String,
        generation: u64,
        result: Result<crate::radio::Station, String>,
    ) {
        let Some(page) = self
            .radio_pages
            .get_mut(&id)
            .filter(|page| page.generation == generation)
        else {
            return;
        };
        match result {
            Ok(station) => {
                page.seed = Some(station.seed.clone());
                let tracks = std::iter::once(&station.seed)
                    .chain(&station.tracks)
                    .cloned()
                    .collect::<Vec<_>>();
                page.station = Loadable::Loaded(station);
                let uris = tracks.iter().map(|track| track.uri.clone()).collect();
                for track in tracks {
                    self.remember_track_recording(&track);
                    if let Some(id) = &track.id {
                        self.track_cache.insert(id.clone(), track);
                    }
                }
                self.request_contains(uris);
                self.play_loaded_radio(&id, generation);
            }
            Err(error) => {
                page.station = Loadable::Failed(error.clone());
                if self.pending_radio_play.as_ref() == Some(&(id, generation)) {
                    self.cancel_radio_play();
                    self.toast(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let root =
            std::env::temp_dir().join(format!("fastpotify-radio-state-{}", std::process::id()));
        let mut app = App::new(
            &Waker::default(),
            AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.backend.set_offline(true);
        app.auth = AuthStatus::Connected {
            username: "test".into(),
        };
        app.local_ready = true;
        app
    }

    #[test]
    fn starting_radio_waits_for_spotifys_playlist_before_loading_audio() {
        let mut app = app();
        app.apply(
            Action::PlayTrackRadio("spotify:track:seed".into()),
            &egui::Context::default(),
        );
        assert!(
            app.backend.take_player_commands().is_empty(),
            "starting radio must resolve Spotify's playlist before loading audio"
        );
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        assert_eq!(app.queue_tab, QueueTab::Queue);
    }

    fn resolved_radio() -> crate::radio::Station {
        crate::radio::Station {
            uri: "spotify:playlist:radio".into(),
            seed: Track {
                uri: "spotify:track:seed".into(),
                ..Default::default()
            },
            tracks: ["first", "seed", "first"]
                .map(|id| Track {
                    id: Some(id.into()),
                    uri: format!("spotify:track:{id}"),
                    ..Default::default()
                })
                .to_vec(),
        }
    }

    #[test]
    fn radio_start_plays_the_resolved_order_without_inserting_or_deduplicating() {
        let mut app = app();
        app.start_radio("spotify:track:seed");
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio("seed".into(), generation, Ok(resolved_radio()));
        let commands = app.backend.take_player_commands();
        let [PlayerCommand::Load(load)] = commands.as_slice() else {
            panic!("the resolved radio must issue one playback request");
        };
        assert_eq!(
            load.uris,
            [
                "spotify:track:first",
                "spotify:track:seed",
                "spotify:track:first"
            ]
        );
        assert!(load.context_uri.is_none());
        assert!(load.play);
        assert_eq!(
            app.playing_context_uri().as_deref(),
            Some("spotify:playlist:radio")
        );
        assert!(app.pending_radio_play.is_none());
    }

    #[test]
    fn radio_start_uses_an_already_loaded_playlist() {
        let mut app = app();
        app.radio_pages.entry("seed".into()).or_default().station =
            Loadable::Loaded(resolved_radio());
        app.start_radio("spotify:track:seed");
        assert!(
            matches!(app.backend.take_player_commands().as_slice(), [PlayerCommand::Load(load)] if load.uris.len() == 3)
        );
        assert!(app.pending_radio_play.is_none());
    }

    #[test]
    fn late_radio_results_cannot_override_newer_playback_controls() {
        for action in [
            Action::PlayUris {
                uris: vec!["spotify:track:other".into()],
                index: 0,
            },
            Action::TogglePlay,
            Action::Next,
            Action::Previous,
            Action::Seek(1000),
            Action::Transfer("another-device".into()),
            Action::SignOut,
        ] {
            let mut app = app();
            app.start_radio("spotify:track:seed");
            let generation = app.radio_pages["seed"].generation;
            app.apply(action, &egui::Context::default());
            app.backend.take_player_commands();
            app.receive_radio("seed".into(), generation, Ok(resolved_radio()));
            assert!(
                app.backend.take_player_commands().is_empty(),
                "a superseded radio must not start playback"
            );
            assert!(app.pending_radio_play.is_none());
        }
    }

    #[test]
    fn only_the_latest_radio_start_can_play() {
        let mut app = app();
        app.start_radio("spotify:track:seed");
        let first = app.radio_pages["seed"].generation;
        app.start_radio("spotify:track:other");
        let second = app.radio_pages["other"].generation;
        app.receive_radio("seed".into(), first, Ok(resolved_radio()));
        assert!(app.backend.take_player_commands().is_empty());
        app.receive_radio("other".into(), second, Ok(resolved_radio()));
        assert!(matches!(
            app.backend.take_player_commands().as_slice(),
            [PlayerCommand::Load(_)]
        ));
        app.receive_radio("seed".into(), first, Ok(resolved_radio()));
        assert!(app.backend.take_player_commands().is_empty());
    }

    #[test]
    fn refreshing_a_pending_radio_waits_for_the_new_playlist() {
        let mut app = app();
        app.start_radio("spotify:track:seed");
        let first = app.radio_pages["seed"].generation;
        app.apply(
            Action::Reload(Page::Radio("seed".into())),
            &egui::Context::default(),
        );
        let second = app.radio_pages["seed"].generation;
        assert!(second > first);
        app.receive_radio("seed".into(), first, Ok(resolved_radio()));
        assert!(app.backend.take_player_commands().is_empty());
        app.receive_radio("seed".into(), second, Ok(resolved_radio()));
        assert!(matches!(
            app.backend.take_player_commands().as_slice(),
            [PlayerCommand::Load(_)]
        ));
    }

    #[test]
    fn failed_radio_start_reports_the_error_and_can_retry() {
        let mut app = app();
        app.start_radio("spotify:track:seed");
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio("seed".into(), generation, Err("radio unavailable".into()));
        assert!(app.pending_radio_play.is_none());
        assert!(!app.any_play_pending());
        assert!(app.backend.take_player_commands().is_empty());
        assert!(!app.toasts.is_empty());
        app.start_radio("spotify:track:seed");
        assert!(app.radio_pages["seed"].generation > generation);
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
    }

    fn text_position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_position(shape, label)),
            _ => None,
        }
    }

    fn browse_from_menu(app: &mut App, ctx: &egui::Context, track: Track) {
        let item = PlayableItem::Track(track);
        let mut draw = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| crate::ui::widgets::item_menu(ui, app, &item, None, None),
            );
            output.textures_delta.clear();
            output
        };
        let pos = draw(vec![])
            .shapes
            .iter()
            .find_map(|shape| text_position(&shape.shape, "Go to song radio"))
            .expect("the browse action is visible");
        for pressed in [true, false] {
            draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        app.apply_actions(ctx);
        assert_eq!(app.page(), &Page::Radio("seed".into()));
    }

    fn radio_frame(app: &mut App, ctx: &egui::Context) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1240.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                crate::ui::radio::show(app, ui, "seed");
            },
        );
        output.textures_delta.clear();
        output
    }

    fn assert_header(output: &egui::FullOutput, title: &str) {
        for label in [title, "Seed artist"] {
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| text_position(&shape.shape, label).is_some()),
                "the radio header must show {label}"
            );
        }
    }

    fn seed_track() -> Track {
        Track {
            // The menu already accepts tracks identified by URI alone.
            uri: "spotify:track:seed".into(),
            name: "Seed song".into(),
            artists: vec![crate::api::models::ArtistRef {
                name: "Seed artist".into(),
                ..Default::default()
            }],
            album: Some(crate::api::models::Album {
                images: vec![
                    crate::api::models::Image {
                        url: "bytes://small.svg".into(),
                        width: Some(64),
                        height: Some(64),
                    },
                    crate::api::models::Image {
                        url: "bytes://large.svg".into(),
                        width: Some(300),
                        height: Some(300),
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn radio_header_survives_loading_error_retry_and_cache_eviction() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        assert!(app.track_cache.is_empty());
        browse_from_menu(&mut app, &ctx, seed_track());
        app.track_cache.clear();
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        assert_header(&radio_frame(&mut app, &ctx), "Seed song Radio");
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio("seed".into(), generation, Err("Still connecting".into()));
        assert_header(&radio_frame(&mut app, &ctx), "Seed song Radio");
        app.reload(Page::Radio("seed".into()));
        assert_header(&radio_frame(&mut app, &ctx), "Seed song Radio");
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio(
            "seed".into(),
            generation,
            Ok(crate::radio::Station {
                uri: "spotify:playlist:radio".into(),
                seed: Track {
                    id: Some("seed".into()),
                    name: "Updated song".into(),
                    ..seed_track()
                },
                tracks: vec![],
            }),
        );
        assert_eq!(app.track_cache["seed"].name, "Updated song");
        app.track_cache.clear();
        app.reload(Page::Radio("seed".into()));
        assert_header(&radio_frame(&mut app, &ctx), "Updated song Radio");
        assert!(app.optimistic_playing.is_none());
        assert!(app.queued_play.is_none());
        assert!(matches!(app.queue, Loadable::NotLoaded));
    }

    #[test]
    fn radio_header_requests_only_the_preferred_cover_when_nothing_is_cached() {
        use egui::load::{BytesLoadResult, BytesLoader, BytesPoll};
        use std::sync::{Arc, Mutex};

        struct PendingArtwork(Arc<Mutex<Vec<String>>>);
        impl BytesLoader for PendingArtwork {
            fn id(&self) -> &str {
                "radio-test::PendingArtwork"
            }
            fn load(&self, _: &egui::Context, uri: &str) -> BytesLoadResult {
                if !matches!(uri, "bytes://small.svg" | "bytes://large.svg") {
                    return Err(egui::load::LoadError::NotSupported);
                }
                self.0.lock().unwrap().push(uri.to_owned());
                Ok(BytesPoll::Pending { size: None })
            }
            fn forget(&self, _: &str) {}
            fn forget_all(&self) {}
            fn byte_size(&self) -> usize {
                0
            }
        }

        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        egui_extras::install_image_loaders(&ctx);
        let requests = Arc::new(Mutex::new(Vec::new()));
        ctx.add_bytes_loader(Arc::new(PendingArtwork(Arc::clone(&requests))));
        browse_from_menu(&mut app, &ctx, seed_track());
        radio_frame(&mut app, &ctx);
        let mut requested = requests.lock().unwrap().clone();
        requested.sort();
        requested.dedup();
        assert_eq!(requested, ["bytes://large.svg"]);
    }

    #[test]
    fn radio_header_uses_available_cover_until_larger_art_arrives() {
        const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#ff0088"/></svg>"##;
        for downloaded in [false, true] {
            let mut app = app();
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            egui_extras::install_image_loaders(&ctx);
            let mut track = seed_track();
            let small_url = if downloaded {
                use std::io::{BufRead, BufReader, Write};
                use std::time::{Duration, Instant};
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let url = format!("http://{}/small.svg", listener.local_addr().unwrap());
                let server = std::thread::spawn(move || {
                    let (mut stream, _) = listener.accept().unwrap();
                    for line in BufReader::new(&mut stream).lines() {
                        if line.unwrap().is_empty() {
                            break;
                        }
                    }
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", SVG.len()).unwrap();
                    stream.write_all(SVG).unwrap();
                });
                ctx.add_bytes_loader(std::sync::Arc::new(app.backend.art().clone()));
                egui::load::BytesLoader::load(app.backend.art(), &ctx, &url).unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !app.backend.art().is_ready(&url) {
                    assert!(Instant::now() < deadline, "fixture artwork did not load");
                    std::thread::sleep(Duration::from_millis(10));
                }
                server.join().unwrap();
                track.album.as_mut().unwrap().images[0].url = url.clone();
                url
            } else {
                ctx.include_bytes("bytes://small.svg", SVG);
                "bytes://small.svg".to_owned()
            };
            browse_from_menu(&mut app, &ctx, track.clone());
            // Seed the existing fallback too, isolating image selection from navigation.
            app.track_cache.insert("seed".into(), track);
            fn texture(ctx: &egui::Context, uri: &str) -> egui::TextureId {
                match egui::Image::new(uri)
                    .load_for_size(ctx, egui::Vec2::splat(212.0))
                    .unwrap()
                {
                    egui::load::TexturePoll::Ready { texture } => texture.id,
                    _ => panic!("included artwork must be ready"),
                }
            }
            fn painted(shape: &egui::Shape, texture: egui::TextureId) -> bool {
                match shape {
                    egui::Shape::Mesh(mesh) => mesh.texture_id == texture,
                    egui::Shape::Rect(rect) => rect.fill_texture_id() == texture,
                    egui::Shape::Vec(shapes) => shapes.iter().any(|shape| painted(shape, texture)),
                    _ => false,
                }
            }
            let small = texture(&ctx, &small_url);
            let output = radio_frame(&mut app, &ctx);
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| painted(&shape.shape, small)),
                "keep drawing the available cover while the larger image is pending"
            );
            // The hero releases source bytes after making a texture. It must still
            // reuse that texture on the next frame without another request.
            let output = radio_frame(&mut app, &ctx);
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| painted(&shape.shape, small))
            );
            ctx.include_bytes("bytes://large.svg", SVG);
            let large = texture(&ctx, "bytes://large.svg");
            assert_ne!(small, large);
            let output = radio_frame(&mut app, &ctx);
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| painted(&shape.shape, large)),
                "upgrade to the preferred cover when it arrives"
            );
        }
    }

    #[test]
    fn reloading_radio_ignores_the_previous_request_and_preserves_playback() {
        let mut app = app();
        app.open(Page::Radio("seed".into()));
        let first = app.radio_pages["seed"].generation;
        app.reload(Page::Radio("seed".into()));
        let second = app.radio_pages["seed"].generation;
        assert_ne!(first, second);
        app.receive_radio("seed".into(), first, Err("old request".into()));
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        app.receive_radio(
            "seed".into(),
            second,
            Ok(crate::radio::Station {
                uri: "spotify:playlist:radio".into(),
                seed: Track {
                    name: "Seed song".into(),
                    ..Default::default()
                },
                tracks: vec![Track {
                    uri: "spotify:track:next".into(),
                    name: "Next song".into(),
                    ..Default::default()
                }],
            }),
        );
        assert_eq!(
            app.radio_pages["seed"].station.get().unwrap().tracks[0].name,
            "Next song"
        );
        assert!(app.optimistic_playing.is_none());
        assert!(app.assumed_context.is_none());
        assert!(app.queued_play.is_none());
        assert!(matches!(app.queue, Loadable::NotLoaded));
    }

    #[test]
    fn refreshing_radio_clears_selection_from_the_previous_songs() {
        let mut app = app();
        let page = Page::Radio("seed".into());
        app.open(page.clone());
        app.pick_row(&page, "original|50", 2, RowPick::Only, 50);
        assert!(app.picked_rows(&page).is_some());
        app.reload(page.clone());
        assert!(app.picked_rows(&page).is_none());
    }

    #[test]
    fn radio_recognizes_a_saved_recording_from_another_release() {
        let mut app = app();
        let original = Track {
            uri: "spotify:track:original".into(),
            external_ids: crate::api::models::ExternalIds {
                isrc: Some("GBUM71029604".into()),
            },
            ..Default::default()
        };
        app.remember_track_recording(&original);
        app.set_saved_state(original.uri.clone(), true);
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio(
            "seed".into(),
            generation,
            Ok(crate::radio::Station {
                uri: "spotify:playlist:radio".into(),
                seed: Track::default(),
                tracks: vec![Track {
                    uri: "spotify:track:radio-release".into(),
                    ..original.clone()
                }],
            }),
        );
        assert_eq!(app.is_saved("spotify:track:radio-release"), Some(true));
        assert_eq!(
            app.saved_toggle_targets("spotify:track:radio-release"),
            vec![original.uri]
        );
    }

    #[test]
    fn restored_radio_retries_when_the_playback_session_becomes_ready() {
        let mut app = app();
        app.local_ready = false;
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio(
            "seed".into(),
            generation,
            Err("Playback is connecting".into()),
        );
        app.handle_playback(LocalPlayback::Ready {
            device_id: "local".into(),
        });
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        assert!(app.radio_pages["seed"].generation > generation);
        assert!(app.queued_play.is_none());
    }

    #[test]
    fn a_radio_error_is_visible_and_retry_can_load_it() {
        let mut app = app();
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio("seed".into(), generation, Err("Set up playback".into()));
        assert!(
            matches!(&app.radio_pages["seed"].station, Loadable::Failed(error) if error == "Set up playback")
        );
        app.reload(Page::Radio("seed".into()));
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        assert!(app.radio_pages["seed"].generation > generation);
    }

    #[test]
    fn a_radio_response_after_sign_out_is_discarded() {
        let mut app = app();
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.handle_auth(AuthStatus::SignedOut);
        app.receive_radio(
            "seed".into(),
            generation,
            Ok(crate::radio::Station::default()),
        );
        assert!(app.radio_pages.is_empty());
    }

    #[test]
    fn radio_controls_browse_play_and_refresh_independently() {
        let mut app = app();
        let item = PlayableItem::Track(Track {
            id: Some("seed".into()),
            uri: "spotify:track:seed".into(),
            name: "Seed song".into(),
            ..Default::default()
        });
        for label in ["Go to song radio", "Start song radio", "Refresh radio"] {
            app.radio_pages.insert(
                "seed".into(),
                crate::radio::RadioPage {
                    station: Loadable::Loaded(crate::radio::Station {
                        uri: "spotify:playlist:radio".into(),
                        seed: match &item {
                            PlayableItem::Track(track) => track.clone(),
                            _ => unreachable!(),
                        },
                        tracks: vec![],
                    }),
                    generation: 1,
                    ..Default::default()
                },
            );
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let mut draw = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(500.0, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        if label == "Refresh radio" {
                            crate::ui::radio::show(&mut app, ui, "seed");
                        } else {
                            crate::ui::widgets::item_menu(ui, &mut app, &item, None, None);
                        }
                    },
                );
                output.textures_delta.clear();
                output
            };
            let output = draw(vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| text_position(&shape.shape, label))
                .expect("the menu item is present");
            for pressed in [true, false] {
                draw(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            let actions = std::mem::take(&mut app.actions);
            assert_eq!(actions.len(), 1);
            if label == "Refresh radio" {
                assert!(matches!(&actions[0], Action::Reload(Page::Radio(id)) if id == "seed"));
            } else if label == "Go to song radio" {
                assert!(
                    matches!(&actions[0], Action::OpenRadio(track) if track.uri == "spotify:track:seed")
                );
            } else {
                assert!(
                    matches!(&actions[0], Action::PlayTrackRadio(uri) if uri == "spotify:track:seed")
                );
            }
        }
    }
}
