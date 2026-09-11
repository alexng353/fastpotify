//! Read-only song-radio resolution through the existing librespot session.

use anyhow::{Context as _, Result, ensure};
use librespot_core::{Session, SpotifyUri};
use librespot_metadata::{Metadata, Playlist, Track as MetadataTrack};
use serde::Deserialize;

use crate::api::models::{Album, ArtistRef, ExternalIds, Image, Track};
use crate::model::Loadable;

#[derive(Clone, Debug, Default)]
pub struct Station {
    pub uri: String,
    pub seed: Track,
    pub tracks: Vec<Track>,
}

#[derive(Default)]
pub struct RadioPage {
    /// Keep the header independent of recommendation requests and cache eviction.
    pub seed: Option<Track>,
    pub station: Loadable<Station>,
    pub generation: u64,
}

/// Reading a radio playlist never activates Connect or loads audio.
pub async fn resolve(session: &Session, seed: &str) -> Result<Station> {
    let seed_uri = SpotifyUri::from_uri(&format!("spotify:track:{seed}"))?;
    // The desktop client's Go to song radio resolves an inspired-by playlist.
    // A station context can return a different selection, even for the same seed.
    let response = session
        .spclient()
        .request_as_json(
            &reqwest::Method::GET,
            &format!(
                "/inspiredby-mix/v2/seed_to_playlist/{}?response-format=json",
                seed_uri.to_uri()?
            ),
            None,
            None,
        )
        .await
        .context("Couldn't load song radio")?;
    let playlist_uri = playlist_uri(&response)?;
    let playlist = Playlist::get(session, &playlist_uri)
        .await
        .context("Couldn't load the radio playlist")?;
    let uris = playlist_uris(&playlist)?;
    let image_url = session
        .get_user_attribute("image-url")
        .unwrap_or_else(|| "https://i.scdn.co/image/{file_id}".into());
    let seed = metadata_track(MetadataTrack::get(session, &seed_uri).await?, &image_url)?;
    let mut tracks = Vec::with_capacity(uris.len());
    // Bound metadata concurrency, and retain Spotify's order despite responses
    // arriving out of order. These requests do not consume the shared Web API quota.
    for chunk in uris.chunks(4) {
        let mut tasks = tokio::task::JoinSet::new();
        for (index, uri) in chunk.iter().cloned().enumerate() {
            let session = session.clone();
            tasks.spawn(async move { (index, MetadataTrack::get(&session, &uri).await) });
        }
        let mut batch = Vec::with_capacity(chunk.len());
        while let Some(result) = tasks.join_next().await {
            let (index, track) = result?;
            batch.push((index, metadata_track(track?, &image_url)?));
        }
        batch.sort_by_key(|(index, _)| *index);
        tracks.extend(batch.into_iter().map(|(_, track)| track));
    }
    Ok(Station {
        uri: playlist_uri.to_uri()?,
        seed,
        tracks,
    })
}

fn playlist_uri(response: &[u8]) -> Result<SpotifyUri> {
    #[derive(Deserialize)]
    struct Response {
        #[serde(rename = "mediaItems")]
        media_items: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        uri: String,
    }
    let response: Response =
        serde_json::from_slice(response).context("Spotify returned an invalid radio response")?;
    let item = response
        .media_items
        .first()
        .context("Spotify returned no playlist for this radio. Try again.")?;
    let uri = SpotifyUri::from_uri(&item.uri)?;
    ensure!(
        matches!(uri, SpotifyUri::Playlist { .. }),
        "Spotify returned a non-playlist radio result"
    );
    Ok(uri)
}

fn playlist_uris(playlist: &Playlist) -> Result<Vec<SpotifyUri>> {
    ensure!(
        !playlist.contents.is_truncated
            && playlist.contents.position == 0
            && usize::try_from(playlist.length).ok() == Some(playlist.contents.items.len()),
        "Spotify returned an incomplete radio playlist. Try again."
    );
    ensure!(
        !playlist.contents.items.is_empty(),
        "Spotify returned no songs for this radio. Try again."
    );
    playlist
        .tracks()
        .map(|uri| {
            ensure!(
                matches!(uri, SpotifyUri::Track { .. }),
                "Spotify returned a non-song radio item"
            );
            Ok(uri.clone())
        })
        .collect()
}

fn metadata_track(track: MetadataTrack, image_url: &str) -> Result<Track> {
    let artists = track
        .artists
        .iter()
        .map(|artist| {
            Ok(ArtistRef {
                id: Some(artist.id.to_id()?),
                uri: Some(artist.id.to_uri()?),
                name: artist.name.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let images = track
        .album
        .covers
        .iter()
        .map(|image| Image {
            url: image_url.replace("{file_id}", &image.id.to_string()),
            width: u32::try_from(image.width).ok(),
            height: u32::try_from(image.height).ok(),
        })
        .collect();
    Ok(Track {
        id: Some(track.id.to_id()?),
        uri: track.id.to_uri()?,
        name: track.name,
        duration_ms: u32::try_from(track.duration).unwrap_or_default(),
        explicit: track.is_explicit,
        external_ids: ExternalIds {
            isrc: track
                .external_ids
                .iter()
                .find(|id| id.external_type.eq_ignore_ascii_case("isrc"))
                .map(|id| id.id.clone()),
        },
        artists,
        album: Some(Album {
            id: track.album.id.to_id()?,
            uri: track.album.id.to_uri()?,
            name: track.album.name,
            images,
            ..Album::default()
        }),
        track_number: u32::try_from(track.number).ok(),
        disc_number: u32::try_from(track.disc_number).ok(),
        popularity: u8::try_from(track.popularity).ok(),
        ..Track::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::playlist4_external::{Item, ListItems, SelectedListContent};

    #[test]
    fn radio_metadata_preserves_recording_identity() {
        use librespot_protocol::metadata;
        let message = metadata::Track {
            gid: Some(vec![1; 16]),
            album: Some(metadata::Album {
                gid: Some(vec![2; 16]),
                date: Some(metadata::Date {
                    year: Some(2020),
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            })
            .into(),
            external_id: vec![metadata::ExternalId {
                type_: Some("isrc".into()),
                id: Some("GBUM71029604".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let track =
            metadata_track(MetadataTrack::try_from(&message).unwrap(), "{file_id}").unwrap();
        assert_eq!(track.recording_key().as_deref(), Some("isrc:GBUM71029604"));
    }

    fn playlist(uris: &[&str]) -> Playlist {
        Playlist::parse(
            &SelectedListContent {
                length: Some(uris.len() as i32),
                contents: Some(ListItems {
                    pos: Some(0),
                    truncated: Some(false),
                    items: uris
                        .iter()
                        .map(|uri| Item {
                            uri: Some((*uri).into()),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            },
            &SpotifyUri::from_uri("spotify:playlist:37i9dQZF1E8CaBx7klrZT9").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn radio_uses_the_playlist_from_spotifys_response() {
        let response =
            br#"{"mediaItems":[{"uri":"spotify:playlist:37i9dQZF1E8CaBx7klrZT9"}],"total":1}"#;
        assert_eq!(
            playlist_uri(response).unwrap().to_uri().unwrap(),
            "spotify:playlist:37i9dQZF1E8CaBx7klrZT9"
        );
    }

    #[test]
    fn missing_or_invalid_radio_playlists_are_errors() {
        for response in [
            br#"{"mediaItems":[]}"#.as_slice(),
            br#"{"mediaItems":[{"uri":"spotify:track:6hkOqJ5mE093AQf2lbZnsG"}]}"#,
            br#"{"mediaItems":[{"uri":"invalid"}]}"#,
            br#"{"mediaItems":[{}]}"#,
            br#"{}"#,
        ] {
            assert!(playlist_uri(response).is_err());
        }
    }

    #[test]
    fn radio_preserves_the_playlist_order_and_duplicate_entries() {
        let uris = [
            "spotify:track:5Ohxk2dO5COHF1krpoPigN",
            "spotify:track:6hkOqJ5mE093AQf2lbZnsG",
            "spotify:track:5Ohxk2dO5COHF1krpoPigN",
        ];
        let playlist = playlist(&uris);
        assert_eq!(
            playlist_uris(&playlist)
                .unwrap()
                .iter()
                .map(|uri| uri.to_uri().unwrap())
                .collect::<Vec<_>>(),
            uris
        );
    }

    #[test]
    fn empty_truncated_or_non_song_radio_playlists_are_errors() {
        assert!(playlist_uris(&playlist(&[])).is_err());
        assert!(playlist_uris(&playlist(&["spotify:album:6hkOqJ5mE093AQf2lbZnsG"])).is_err());
        let full = playlist(&["spotify:track:6hkOqJ5mE093AQf2lbZnsG"]);
        let mut truncated = full.clone();
        truncated.contents.is_truncated = true;
        assert!(playlist_uris(&truncated).is_err());
        let mut missing = full.clone();
        missing.length = 2;
        assert!(playlist_uris(&missing).is_err());
        let mut offset = full;
        offset.contents.position = 1;
        assert!(playlist_uris(&offset).is_err());
    }
}
