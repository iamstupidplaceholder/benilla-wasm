//! Pending-aware dimensions only. The production decoder and Lua layout are reused.

use benilla_assets::{decode_sprite_from_source, SpriteDecode, SpriteSource};
use benilla_ui::script::UiScript;
use bevy::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Answers {
    pending: HashSet<String>,
    sizes: HashMap<String, Option<(u32, u32)>>,
    generation: u64,
}

#[derive(Resource, Clone, Default)]
pub(super) struct SizeBroker(Arc<Mutex<Answers>>);

impl SizeBroker {
    pub(super) fn probe(&self, path: &str) -> Option<(u32, u32)> {
        let mut answers = self.0.lock().expect("texture-size answers poisoned");
        if let Some(size) = answers.sizes.get(path) {
            return *size;
        }
        answers.pending.insert(path.to_owned());
        None
    }

    pub(super) fn poll(&self, source: &mut impl SpriteSource) {
        let pending: Vec<_> = self
            .0
            .lock()
            .expect("texture-size answers poisoned")
            .pending
            .iter()
            .cloned()
            .collect();
        for path in pending {
            self.complete(&path, decode_sprite_from_source(source, &path));
        }
    }

    /// A declared boot file is already resident when its first Lua getter runs.
    /// Dynamic paths queue instead; never pretend Pending is a completed miss.
    pub(super) fn probe_resident(
        &self,
        path: &str,
        source: &mut impl SpriteSource,
    ) -> Option<(u32, u32)> {
        {
            let answers = self.0.lock().expect("texture-size answers poisoned");
            if let Some(size) = answers.sizes.get(path) {
                return *size;
            }
            if answers.pending.contains(path) {
                return None;
            }
        }
        self.complete(path, decode_sprite_from_source(source, path));
        self.probe(path)
    }

    fn complete(&self, path: &str, decoded: SpriteDecode) {
        let size = match decoded {
            SpriteDecode::Pending => return,
            SpriteDecode::Ready(w, h, _) => Some((w, h)),
            SpriteDecode::Failed { undecodable } => {
                if undecodable.is_empty() {
                    warn!("texture-size miss: '{path}' does not resolve in the host byte source");
                } else {
                    warn!(
                        "texture-size miss: '{path}' resolves but will not decode ({})",
                        undecodable.join(", ")
                    );
                }
                None
            }
        };
        let mut answers = self.0.lock().expect("texture-size answers poisoned");
        answers.sizes.insert(path.to_owned(), size);
        answers.pending.remove(path);
        answers.generation = answers.generation.wrapping_add(1);
    }

    fn generation(&self) -> u64 {
        self.0
            .lock()
            .expect("texture-size answers poisoned")
            .generation
    }
}

#[cfg(feature = "port-gates")]
pub fn verify_async_texture_size_probe() {
    use benilla_assets::SpriteRead;
    use benilla_formats::{FileSource, MemoryFileSource};
    struct Source {
        pending: bool,
        files: MemoryFileSource,
        reads: usize,
    }
    impl SpriteSource for Source {
        fn read_installed(&mut self, path: &str) -> SpriteRead {
            self.reads += 1;
            if self.pending && path == "interface\\crest.blp" {
                return SpriteRead::Pending;
            }
            self.files
                .read_file(path)
                .ok()
                .map_or(SpriteRead::Missing, SpriteRead::Bytes)
        }
    }
    let mut tga = vec![0; 18];
    tga[2] = 2;
    tga[12] = 2;
    tga[14] = 3;
    tga[16] = 32;
    tga[17] = 0x28;
    tga.extend_from_slice(&[30, 20, 10, 255].repeat(6));
    let mut source = Source {
        pending: true,
        files: MemoryFileSource::new([("Interface\\Crest.blp".to_owned(), tga.clone())]),
        reads: 0,
    };
    // Real resident bytes answer inside the first Lua chunk, before any update
    // or observer can repair a load-time branch using GetWidth/GetHeight.
    let resident_broker = SizeBroker::default();
    let resident_source = std::cell::RefCell::new(Source {
        pending: false,
        files: MemoryFileSource::new([("Interface\\Crest.blp".to_owned(), tga)]),
        reads: 0,
    });
    let mut resident_script = UiScript::new().unwrap();
    resident_script.set_texture_probe(Box::new(|path| path == "Interface\\Crest"));
    resident_script.set_texture_size_probe(Box::new(move |path| {
        resident_broker.probe_resident(path, &mut *resident_source.borrow_mut())
    }));
    resident_script.run(r#"
        local owner = CreateFrame("Frame", "ResidentGateOwner")
        local art = owner:CreateTexture(nil, "ARTWORK")
        assert(art:SetTexture("Interface\\Crest") == 1)
        assert(art:GetWidth() == 2 and art:GetHeight() == 3, "first load-time read must see resident texels")
    "#).unwrap();
    let broker = SizeBroker::default();
    let probe = broker.clone();
    let mut script = UiScript::new().unwrap();
    script.set_screen_size(800.0, 600.0);
    script.set_texture_probe(Box::new(|path| path == "Interface\\Crest"));
    script.set_texture_size_probe(Box::new(move |path| probe.probe(path)));
    script
        .run(
            r#"
        local owner = CreateFrame("Frame", "SizeGateOwner")
        owner:SetPoint("BOTTOMLEFT", 0, 0); owner:SetWidth(100); owner:SetHeight(50)
        SizeGateArt = owner:CreateTexture("SizeGateArt", "ARTWORK")
        assert(SizeGateArt:SetTexture("Interface\\Crest") == 1)
        SizeGateArt:SetPoint("TOPLEFT", 0, 0)
        assert(SizeGateArt:GetWidth() == 0 and SizeGateArt:GetHeight() == 0)
    "#,
        )
        .unwrap();
    script.resolve();
    broker.poll(&mut source);
    assert_eq!(
        broker.generation(),
        0,
        "pending must not publish a missing-size answer"
    );
    assert!(broker.probe("Interface\\Crest").is_none());
    source.pending = false;
    broker.poll(&mut source);
    assert_eq!(broker.probe("Interface\\Crest"), Some((2, 3)));
    let reads = source.reads;
    broker.poll(&mut source);
    assert_eq!(source.reads, reads, "completed size lookup is memoized");

    // The actual session-scoped observer pushes the existing layout invalidation.
    let mut app = App::new();
    app.insert_resource(broker.clone())
        .insert_non_send_resource(script)
        .add_systems(Update, refresh_layout);
    app.update();
    let mut script = app.world_mut().non_send_resource_mut::<UiScript>();
    script.resolve();
    script
        .run("assert(SizeGateArt:GetWidth() == 2 and SizeGateArt:GetHeight() == 3)")
        .unwrap();
    assert!(
        script.extract().iter().any(|q| q
            .rect
            .as_ref()
            .is_some_and(|r| r.right - r.left == 2.0 && r.top - r.bottom == 3.0)),
        "delivered texel dimensions must reach resolved geometry"
    );
    drop(script);
    assert!(broker.probe("absent").is_none());
    broker.poll(&mut source);
    let reads = source.reads;
    assert!(broker.probe("absent").is_none());
    broker.poll(&mut source);
    assert_eq!(
        source.reads, reads,
        "terminal absence is memoized, unlike pending"
    );
}

/// Reuse the existing production full-resolve invalidation, session-scoped like
/// other VM memo pushes. Neither Lua layout nor getter semantics are rewritten.
pub(super) fn refresh_layout(
    broker: Res<SizeBroker>,
    script: Option<NonSendMut<UiScript>>,
    mut seen: Local<crate::ui_script::VmMemo<u64>>,
) {
    let Some(mut script) = script else { return };
    let generation = broker.generation();
    let seen = seen.get(&script);
    if *seen != generation {
        script.force_full_layout_resolve();
        *seen = generation;
    }
}
