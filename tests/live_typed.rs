//! Opt-in real reads of the Rust entry points used by generated bindings.
//! Login flows, account writes and screenshots are excluded.
use qqmusic_api_helper_next::{api, configure, Configuration, HelperError, Platform};

#[test]
#[ignore = "requires QQMUSIC_HELPER_NEXT_DIR and real network/account"]
fn typed_catalogue_and_account_reads() -> Result<(), HelperError> {
    let data_dir = std::env::var("QQMUSIC_HELPER_NEXT_DIR").expect("set the credential directory");
    configure(Configuration {
        data_dir,
        default_platform: Platform::Web,
    });
    let mid = "003w2xz20QlUZt".to_string();
    let artist_mid = "0025NhlN2yWrP4".to_string();
    let song = api::song_detail(mid.clone())?;
    assert_eq!(song.song_mid, mid);
    assert!(song.title.as_ref().is_some_and(|s| !s.is_empty()));
    let album = api::album_detail(Some("0042cH172YJ0mz".into()), None)?;
    assert!(album.title.as_ref().is_some_and(|s| !s.is_empty()));
    let tracks = api::album_tracks(Some("0042cH172YJ0mz".into()), None, 0, 5)?;
    assert!(!tracks.tracks.is_empty());
    let artist = api::artist_detail(artist_mid.clone())?;
    assert_eq!(artist.singer_mid, artist_mid);
    assert!(!artist.description.is_empty());
    let biography = api::fetch_artist_biography("周杰伦".into(), Some(artist_mid.clone()))?;
    assert!(biography
        .description
        .as_ref()
        .is_some_and(|s| !s.is_empty()));
    assert!(!api::artist_songs(artist_mid.clone(), "hot".into(), 1, 5)?.is_empty());
    assert!(!api::artist_albums(artist_mid, "hot".into(), 1, 5)?.is_empty());
    assert!(!api::toplist_categories()?.is_empty());
    assert!(!api::radio_stations()?.is_empty());
    assert!(!api::new_songs(0)?.is_empty());
    let lyric = api::lyric(mid.clone(), song.song_id, true, true)?;
    assert!(lyric.lyric.as_ref().is_some_and(|s| !s.is_empty()));
    let stream = api::resolve_song_url(mid.clone(), None, 0, Some("128".into()))?;
    assert_eq!(stream.song_mid, mid);
    assert!(stream.playable && stream.url.as_ref().is_some_and(|s| !s.is_empty()));
    assert!(!api::liked_songs(1, 5)?.tracks.is_empty());
    assert!(!api::user_playlists(10)?.is_empty());
    assert!(!api::liked_albums(10)?.is_empty());
    assert!(api::component_info()?
        .methods
        .iter()
        .any(|s| s == "fetch_new_albums"));
    api::guard_status()?;
    // Local component settings are per-process; no remote account mutation.
    api::set_rate_limit(true, 60, 60)?;
    api::set_breaker(true, 5, 60, 30)?;
    let raw = api::call_with_platform(
        "fetch_album_detail".into(),
        r#"{"albumMid":"0042cH172YJ0mz"}"#.into(),
        "web".into(),
    )?;
    assert!(serde_json::from_str::<serde_json::Value>(&raw)
        .unwrap()
        .get("detail")
        .is_some());
    println!(
        "typed catalogue/account payloads and raw platform contract verified; no account writes"
    );
    Ok(())
}
