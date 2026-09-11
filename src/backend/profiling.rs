use super::{ApiError, ApiRequest, ApiResponse};
use crate::profiling::LoadTimer;

pub(super) fn start(request: &ApiRequest) -> Option<LoadTimer> {
    let name = match request {
        ApiRequest::Me => "account profile",
        ApiRequest::Devices => "devices",
        ApiRequest::RecentlyPlayed { .. } => "recently played",
        ApiRequest::TopTracks { .. } => "top tracks",
        ApiRequest::TopArtists { .. } => "top artists",
        ApiRequest::Recommendations { .. } => "recommendations",
        ApiRequest::Discover { .. } => "discovery",
        ApiRequest::MyPlaylists { .. } => "playlist library",
        ApiRequest::Playlist { .. } => "playlist",
        ApiRequest::PlaylistItems { .. } => "playlist items",
        ApiRequest::PlaylistSample { .. } => "playlist sample",
        ApiRequest::SavedTracks { .. } => "liked songs",
        ApiRequest::SavedAlbums { .. } => "saved albums",
        ApiRequest::FollowedArtists { .. } => "followed artists",
        ApiRequest::SavedShows { .. } => "saved shows",
        ApiRequest::SavedEpisodes { .. } => "saved episodes",
        ApiRequest::Contains { .. } => "saved status",
        ApiRequest::Search { .. } => "search",
        ApiRequest::Artist { .. } => "artist",
        ApiRequest::ArtistTopTracks { .. } => "artist top tracks",
        ApiRequest::ArtistAlbums { .. } => "artist albums",
        ApiRequest::RelatedArtists { .. } => "related artists",
        ApiRequest::Album { .. } => "album",
        ApiRequest::AlbumTracks { .. } => "album tracks",
        ApiRequest::Show { .. } => "show",
        ApiRequest::ShowEpisodes { .. } => "show episodes",
        ApiRequest::Track { .. } => "track metadata",
        ApiRequest::Episode { .. } => "episode metadata",
        // Routine playback polling and mutations are not content loads.
        _ => return None,
    };
    // Only the read-only variants above reach this point. Debug quoting keeps
    // query newlines escaped; response bodies and grants are never formatted.
    Some(LoadTimer::new(
        name,
        format!("source=api request={request:?}"),
    ))
}

pub(super) fn finish(timer: &mut LoadTimer, response: &ApiResponse) {
    macro_rules! result {
        ($result:expr, $value:ident, $details:expr) => {
            match $result {
                Ok($value) => timer.finish("Loaded", $details),
                Err(error) => timer.finish("Load failed", error_details(error)),
            }
        };
    }
    macro_rules! page {
        ($result:expr) => {
            result!(
                $result,
                page,
                format!(
                    "items={} total={} offset={} has_next={}",
                    page.items.len(),
                    page.total,
                    page.offset,
                    page.next.is_some()
                )
            )
        };
    }
    macro_rules! list {
        ($result:expr) => {
            result!($result, items, format!("items={}", items.len()))
        };
    }
    match response {
        ApiResponse::Me(r) => result!(r, _user, "items=1"),
        ApiResponse::Devices(r) => list!(r),
        ApiResponse::RecentlyPlayed { result: r, .. } => result!(
            r,
            p,
            format!(
                "items={} total={:?} has_next={}",
                p.items.len(),
                p.total,
                p.next.is_some()
            )
        ),
        ApiResponse::TopTracks { result: r, .. } => page!(r),
        ApiResponse::TopArtists { result: r, .. } => list!(r),
        ApiResponse::Recommendations { result: r, .. } => list!(r),
        ApiResponse::Discover { result: r, .. } => list!(r),
        ApiResponse::MyPlaylists { result: r, .. } => page!(r),
        ApiResponse::Playlist { result: r, .. } => {
            result!(r, p, format!("name={:?} total={}", p.name, p.track_total()))
        }
        ApiResponse::PlaylistItems { result: r, .. }
        | ApiResponse::PlaylistSample { result: r, .. } => page!(r),
        ApiResponse::SavedTracks { result: r, .. } => page!(r),
        ApiResponse::SavedAlbums { result: r, .. } => page!(r),
        ApiResponse::FollowedArtists { result: r, .. } => result!(
            r,
            p,
            format!(
                "items={} total={:?} has_next={}",
                p.items.len(),
                p.total,
                p.next.is_some()
            )
        ),
        ApiResponse::SavedShows { result: r, .. } => page!(r),
        ApiResponse::SavedEpisodes { result: r, .. } => page!(r),
        ApiResponse::Contains { result: r, .. } => list!(r),
        ApiResponse::Search { result: r, .. } => result!(
            r,
            s,
            format!(
                "tracks={} artists={} albums={} playlists={} shows={} episodes={}",
                s.tracks.as_ref().map_or(0, |p| p.items.len()),
                s.artists.as_ref().map_or(0, |p| p.items.len()),
                s.albums.as_ref().map_or(0, |p| p.items.len()),
                s.playlists.as_ref().map_or(0, |p| p.items.len()),
                s.shows.as_ref().map_or(0, |p| p.items.len()),
                s.episodes.as_ref().map_or(0, |p| p.items.len())
            )
        ),
        ApiResponse::Artist { result: r, .. } => result!(r, a, format!("name={:?}", a.name)),
        ApiResponse::ArtistTopTracks { result: r, .. } => list!(r),
        ApiResponse::ArtistAlbums { result: r, .. } => page!(r),
        ApiResponse::RelatedArtists { result: r, .. } => list!(r),
        ApiResponse::Album { result: r, .. } => result!(r, a, format!("name={:?}", a.name)),
        ApiResponse::AlbumTracks { result: r, .. } => page!(r),
        ApiResponse::Show { result: r, .. } => result!(r, s, format!("name={:?}", s.name)),
        ApiResponse::ShowEpisodes { result: r, .. } => page!(r),
        ApiResponse::Track { result: r, .. } => result!(r, t, format!("name={:?}", t.name)),
        ApiResponse::Episode { result: r, .. } => result!(r, e, format!("name={:?}", e.name)),
        _ => timer.finish("Load failed", "error=unexpected_response"),
    }
}

fn error_details(error: &ApiError) -> String {
    let kind = match error {
        ApiError::NotSignedIn => "not_signed_in",
        ApiError::Status { .. } => "http",
        ApiError::RateLimited => "rate_limited",
        ApiError::QuotaExhausted => "quota_exhausted",
        ApiError::SignInExpired { .. } => "sign_in_expired",
        ApiError::Network(_) => "network",
        ApiError::Decode(_) => "decode",
    };
    format!("error={kind} http_status={:?}", error.status())
}
