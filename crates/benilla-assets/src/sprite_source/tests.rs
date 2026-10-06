use super::*;
use std::collections::HashMap;

fn tga() -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[2] = 2;
    bytes[12] = 1;
    bytes[14] = 1;
    bytes[16] = 32;
    bytes[17] = 0x28;
    bytes.extend_from_slice(&[30, 20, 10, 255]);
    bytes
}

fn blp() -> Vec<u8> {
    // Direct BLP2 carries the palette even for Raw3, where it is ignored.
    let mut bytes = vec![0; 148 + 1024];
    bytes[..4].copy_from_slice(b"BLP2");
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8] = 3;
    bytes[9] = 8;
    bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&1172u32.to_le_bytes());
    bytes[84..88].copy_from_slice(&4u32.to_le_bytes());
    bytes.extend_from_slice(&[30, 20, 10, 255]);
    bytes
}

#[derive(Default)]
struct Source {
    installed: HashMap<String, Vec<u8>>,
    loose: HashMap<String, Vec<u8>>,
    pending: Option<String>,
    reads: Vec<(bool, String)>,
}

impl SpriteSource for Source {
    fn read_installed(&mut self, path: &str) -> SpriteRead {
        self.reads.push((false, path.into()));
        if self.pending.as_deref() == Some(path) {
            return SpriteRead::Pending;
        }
        self.installed
            .get(path)
            .cloned()
            .map_or(SpriteRead::Missing, SpriteRead::Bytes)
    }
    fn read_loose(&mut self, path: &str) -> SpriteRead {
        self.reads.push((true, path.into()));
        self.loose
            .get(path)
            .cloned()
            .map_or(SpriteRead::Missing, SpriteRead::Bytes)
    }
}

#[test]
fn content_decoding_and_candidate_store_order_are_shared() {
    for (path, bytes) in [("icon.blp", tga()), ("icon.tga", blp())] {
        let mut source = Source::default();
        source.installed.insert(path.into(), bytes);
        assert_eq!(
            decode_sprite_from_source(&mut source, path),
            SpriteDecode::Ready(1, 1, vec![10, 20, 30, 255])
        );
        assert_eq!(source.reads, [(false, path.into())]);
    }
    let mut source = Source::default();
    source.installed.insert("icon.blp".into(), vec![0xff]);
    source.loose.insert("icon.blp".into(), tga());
    assert!(matches!(
        decode_sprite_from_source(&mut source, "Icon"),
        SpriteDecode::Ready(..)
    ));
    assert_eq!(
        source.reads,
        [(false, "icon.blp".into()), (true, "icon.blp".into())]
    );
    source.loose.clear();
    source.installed.insert("icon.tga".into(), tga());
    source.reads.clear();
    assert!(matches!(
        decode_sprite_from_source(&mut source, "Icon"),
        SpriteDecode::Ready(..)
    ));
    assert_eq!(
        source.reads,
        [
            (false, "icon.blp".into()),
            (true, "icon.blp".into()),
            (false, "icon.tga".into())
        ]
    );
}

#[test]
fn pending_is_not_a_miss_and_cannot_select_lower_priority_art() {
    let mut source = Source {
        pending: Some("icon.blp".into()),
        ..Default::default()
    };
    source.loose.insert("icon.blp".into(), tga());
    source.installed.insert("icon.tga".into(), tga());
    assert_eq!(
        decode_sprite_from_source(&mut source, "ICON"),
        SpriteDecode::Pending
    );
    assert_eq!(source.reads, [(false, "icon.blp".into())]);
    source.pending = None;
    assert!(matches!(
        decode_sprite_from_source(&mut source, "icon"),
        SpriteDecode::Ready(..)
    ));
}

#[test]
fn missing_and_undecodable_diagnostics_remain_distinct() {
    let mut source = Source::default();
    assert_eq!(
        decode_sprite_from_source(&mut source, "icon"),
        SpriteDecode::Failed {
            undecodable: vec![]
        }
    );
    source.installed.insert("icon.blp".into(), vec![0xff]);
    source.loose.insert("icon.blp".into(), vec![0x80]);
    assert_eq!(
        decode_sprite_from_source(&mut source, "icon"),
        SpriteDecode::Failed {
            undecodable: vec!["icon.blp".into()]
        }
    );
}

#[test]
fn real_asset_server_delivery_retries_pending_and_falls_back_after_failure() {
    use bevy::{
        asset::{
            io::{file::FileAssetReader, AssetSourceBuilder},
            AssetApp, AssetPlugin,
        },
        prelude::*,
    };
    use std::time::{Duration, Instant};
    let root = std::env::temp_dir().join(format!("benilla-sprite-source-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("icon.tga"), tga()).unwrap();
    let reader_root = root.clone();
    let mut app = App::new();
    app.register_asset_source(
        "mpq",
        AssetSourceBuilder::new(move || Box::new(FileAssetReader::new(reader_root.clone()))),
    );
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), crate::RawFilePlugin));
    let server = app.world().resource::<AssetServer>().clone();
    let mut files = SpriteFiles::default();
    let mut cache = crate::SpriteCache::default();
    let mut images = Assets::<Image>::default();
    assert!(cache
        .sprite_texture(
            &mut files.reader(&server, app.world().resource::<Assets<crate::RawFile>>()),
            "ICON",
            &mut images
        )
        .is_none());
    assert!(
        images.is_empty(),
        "pending bytes must not make fallback art"
    );
    let first = decode_sprite_from_source(
        &mut files.reader(&server, app.world().resource::<Assets<crate::RawFile>>()),
        "ICON",
    );
    assert_eq!(first, SpriteDecode::Pending);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.update();
        let result = decode_sprite_from_source(
            &mut files.reader(&server, app.world().resource::<Assets<crate::RawFile>>()),
            "icon",
        );
        match result {
            SpriteDecode::Pending => {}
            SpriteDecode::Ready(w, h, rgba) => {
                assert_eq!((w, h, rgba), (1, 1, vec![10, 20, 30, 255]));
                let handle = cache
                    .sprite_texture(
                        &mut files
                            .reader(&server, app.world().resource::<Assets<crate::RawFile>>()),
                        "icon",
                        &mut images,
                    )
                    .expect("previous pending must not poison the cache");
                let again = cache
                    .sprite_texture(
                        &mut files
                            .reader(&server, app.world().resource::<Assets<crate::RawFile>>()),
                        "ICON.blp",
                        &mut images,
                    )
                    .unwrap();
                assert_eq!(handle, again);
                assert_eq!(images.len(), 1);
                assert_eq!(
                    images.get(&handle).unwrap().data.as_deref(),
                    Some([10, 20, 30, 255].as_slice())
                );
                break;
            }
            other => panic!("unexpected delivery outcome: {other:?}"),
        }
        assert!(Instant::now() < deadline, "sprite delivery never completed");
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(files);
    drop(app);
    std::fs::remove_file(root.join("icon.tga")).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn all_original_sprite_cache_lanes_retry_pending_and_preserve_identity() {
    use bevy::prelude::*;
    for lane in 0..6 {
        let mut cache = crate::SpriteCache::default();
        let mut images = Assets::<Image>::default();
        let mut source = Source {
            pending: Some("icon.blp".into()),
            ..Default::default()
        };
        let calls = std::cell::Cell::new(0);
        let resample = |src: &[u8], sw, sh, w, h| {
            assert_eq!((sw, sh, w, h), (1, 1, 1, 1));
            calls.set(calls.get() + 1);
            src.to_vec()
        };
        let load = |cache: &mut crate::SpriteCache,
                    source: &mut Source,
                    images: &mut Assets<Image>| match lane {
            0 => cache.sprite_texture(source, "icon", images),
            1 => cache.sprite_texture_wrapped(source, "icon", (true, false), images),
            2 => cache.portrait_texture(source, "icon", images),
            3 => cache.emblem_mask_texture(source, "icon", images),
            4 => cache.mask_texture(source, "icon", images),
            5 => cache.resampled_sprite(source, "icon", (1, 1), images, resample),
            _ => unreachable!(),
        };
        assert!(load(&mut cache, &mut source, &mut images).is_none());
        assert!(
            cache.has_pending(),
            "pending conversion must prevent settled/spliced paint"
        );
        assert!(images.is_empty());
        assert_eq!(calls.get(), 0);
        source.pending = None;
        source.installed.insert("icon.blp".into(), tga());
        cache.begin_paint();
        let handle = load(&mut cache, &mut source, &mut images).unwrap();
        assert!(
            !cache.has_pending(),
            "completed visible paths allow paint to settle"
        );
        let reads = source.reads.len();
        assert_eq!(
            load(&mut cache, &mut source, &mut images),
            Some(handle.clone())
        );
        assert_eq!(
            source.reads.len(),
            reads,
            "cache hit must not re-read bytes"
        );
        assert_eq!(images.len(), 1);
        let image = images.get(&handle).unwrap();
        if lane == 3 {
            assert_eq!(image.data.as_deref(), Some([255, 255, 255, 255].as_slice()));
        }
        assert_eq!(image.texture_descriptor.format.is_srgb(), lane != 4);
        if lane == 5 {
            assert_eq!(calls.get(), 1);
        }
    }
}

#[test]
fn hidden_pending_paths_do_not_pin_later_paint_passes() {
    use bevy::prelude::*;
    let mut cache = crate::SpriteCache::default();
    let mut images = Assets::<Image>::default();
    let mut source = Source {
        pending: Some("icon.blp".into()),
        ..Default::default()
    };
    assert!(cache
        .sprite_texture(&mut source, "icon", &mut images)
        .is_none());
    assert!(cache.has_pending());
    cache.begin_paint();
    // A full pass where that region was hidden/removed does not ask its path.
    assert!(!cache.has_pending());
    assert!(images.is_empty());
}

#[test]
fn terminal_misses_wrap_variants_resampling_and_generated_cache_rules_survive() {
    use bevy::prelude::*;
    let mut cache = crate::SpriteCache::default();
    let mut images = Assets::<Image>::default();
    let mut source = Source::default();
    assert!(cache
        .sprite_texture(&mut source, "icon", &mut images)
        .is_none());
    let reads = source.reads.len();
    source.installed.insert("icon.blp".into(), tga());
    assert!(cache
        .sprite_texture(&mut source, "ICON.blp", &mut images)
        .is_none());
    assert_eq!(
        source.reads.len(),
        reads,
        "terminal miss stays cached until original invalidation edge"
    );
    cache.clear_misses();
    let plain = cache
        .sprite_texture(&mut source, "icon", &mut images)
        .unwrap();
    let wrap_u = cache
        .sprite_texture_wrapped(&mut source, "icon", (true, false), &mut images)
        .unwrap();
    let wrap_both = cache
        .sprite_texture_tiled(&mut source, "icon", &mut images)
        .unwrap();
    assert_ne!(plain, wrap_u);
    assert_ne!(wrap_u, wrap_both);
    let resized = cache
        .resampled_sprite(
            &mut source,
            "icon",
            (1, 1),
            &mut images,
            |rgba, _, _, _, _| rgba.to_vec(),
        )
        .unwrap();
    let reads = source.reads.len();
    let larger = cache
        .resampled_sprite(
            &mut source,
            "icon",
            (2, 2),
            &mut images,
            |rgba, sw, sh, w, h| {
                assert_eq!((sw, sh, w, h), (1, 1, 2, 2));
                rgba.repeat(4)
            },
        )
        .unwrap();
    assert_ne!(resized, larger);
    assert_eq!(
        source.reads.len(),
        reads,
        "resize reuses decoded source pixels"
    );
    cache.clear_misses();
    assert_eq!(
        cache.sprite_texture(&mut source, "icon", &mut images),
        Some(plain)
    );
    assert_eq!(
        source.reads.len(),
        reads,
        "root-change invalidation preserves hits"
    );
    let generated =
        cache.generated_sprite("test-wheel", &mut images, || (1, 1, vec![1, 2, 3, 255]));
    assert_eq!(
        generated,
        cache.generated_sprite("test-wheel", &mut images, || panic!(
            "generated sprite rebuilt on hit"
        ))
    );
}
