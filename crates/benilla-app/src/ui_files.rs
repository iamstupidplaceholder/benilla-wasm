//! Delivery planning only. Does not execute Lua, create frames or reorder the manifest.
//! The existing parser identifies file references; the existing loader joins their paths.

use benilla_ui::framexml::{Element, ParsedDocument, ScriptRef, TopLevel};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct UiFilePlan {
    files: HashMap<String, (String, Option<Vec<u8>>)>,
    presence: Option<benilla_formats::ChainPresenceIndex>,
    addons: benilla_interface_files::AddonCatalog,
    textures: HashSet<String>,
    resolved_textures: HashSet<String>,
    fonts: HashSet<String>,
}

impl UiFilePlan {
    pub(crate) fn new(paths: impl IntoIterator<Item = String>) -> Self {
        let mut plan = Self::default();
        for path in paths {
            plan.request(path);
        }
        plan
    }

    fn request(&mut self, path: String) {
        self.files
            .entry(benilla_assets::normalize_path(&path))
            .or_insert((path, None));
    }

    pub(crate) fn pending(&self) -> impl Iterator<Item = &str> {
        self.files
            .values()
            .filter_map(|(path, bytes)| bytes.is_none().then_some(path.as_str()))
    }

    pub(crate) fn accept_presence(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.presence = Some(
            benilla_formats::ChainPresenceIndex::decode(bytes)
                .map_err(|error| error.to_string())?,
        );
        self.request_art();
        Ok(())
    }

    pub(crate) fn accept_addons(&mut self, addons: benilla_interface_files::AddonCatalog) {
        // File admission only. The original load walk decides enable/version,
        // dependencies, LoadOnDemand and execution order after publication.
        let mut roots: Vec<_> = addons.manifests().map(|(_, toc)| toc.to_owned()).collect();
        roots.extend(
            addons
                .files()
                .filter(|path| {
                    let lower = path.to_ascii_lowercase();
                    lower.ends_with("/bindings.xml")
                        || [".blp", ".tga", ".ttf", ".otf"]
                            .iter()
                            .any(|ext| lower.ends_with(ext))
                })
                .map(str::to_owned),
        );
        self.addons = addons;
        for path in roots {
            self.request(path);
        }
        self.request_art();
    }

    fn contains(&self, path: &str) -> bool {
        self.addons.contains(path)
            || self
                .presence
                .as_ref()
                .is_some_and(|index| index.contains(path))
    }

    /// Admit optional archive roots only after exact presence is known.
    pub(crate) fn request_present(&mut self, paths: impl IntoIterator<Item = String>) {
        let present: Vec<_> = paths
            .into_iter()
            .filter(|path| self.contains(path))
            .collect();
        for path in present {
            self.request(path);
        }
    }

    #[cfg(feature = "port-gates")]
    fn art_candidates(&self, contains: impl Fn(&str) -> bool) -> Vec<String> {
        self.art_discovery(contains).0
    }

    fn art_discovery(&self, contains: impl Fn(&str) -> bool) -> (Vec<String>, Vec<String>) {
        struct Resident<'a, F> {
            files: &'a HashMap<String, (String, Option<Vec<u8>>)>,
            contains: F,
            pending: Vec<String>,
        }
        impl<F: Fn(&str) -> bool> benilla_assets::SpriteSource for Resident<'_, F> {
            fn read_installed(&mut self, path: &str) -> benilla_assets::SpriteRead {
                if !(self.contains)(path) {
                    return benilla_assets::SpriteRead::Missing;
                }
                if let Some((_, Some(bytes))) =
                    self.files.get(&benilla_assets::normalize_path(path))
                {
                    return benilla_assets::SpriteRead::Bytes(bytes.clone());
                }
                self.pending.push(path.to_owned());
                benilla_assets::SpriteRead::Pending
            }
        }
        let mut source = Resident {
            files: &self.files,
            contains: &contains,
            pending: Vec::new(),
        };
        let mut completed = Vec::new();
        for path in &self.textures {
            if self.resolved_textures.contains(path) {
                continue;
            }
            // Ask the real decoder to choose its next required candidate. A
            // valid first candidate must not require an unused fallback file.
            if !matches!(
                benilla_assets::decode_sprite_from_source(&mut source, path),
                benilla_assets::SpriteDecode::Pending
            ) {
                completed.push(path.clone());
            }
        }
        source.pending.extend(
            self.fonts
                .iter()
                .filter(|path| {
                    contains(path)
                        && !matches!(
                            self.files.get(&benilla_assets::normalize_path(path)),
                            Some((_, Some(_)))
                        )
                })
                .cloned(),
        );
        (source.pending, completed)
    }

    fn request_art(&mut self) {
        if self.presence.is_none() {
            return;
        }
        let (paths, completed) = self.art_discovery(|path| self.contains(path));
        // The archive presence and admitted bytes are immutable for this plan.
        // A terminal decoder verdict cannot change when an unrelated XML/Lua
        // file arrives. Never repeat full image decoding for those paths.
        self.resolved_textures.extend(completed);
        for path in paths {
            self.request(path);
        }
    }

    fn discover_art(&mut self, doc: &ParsedDocument) {
        fn visit(el: &Element, textures: &mut HashSet<String>, fonts: &mut HashSet<String>) {
            // Delivery hints only, never widget construction. Script/Include file=
            // names are code, not art; font= is the loader's literal font path.
            if !el.tag.eq_ignore_ascii_case("Script") && !el.tag.eq_ignore_ascii_case("Include") {
                let texture = el.tag.to_ascii_lowercase().ends_with("texture");
                for attr in ["file", "bgFile", "edgeFile"] {
                    if attr == "file" && !texture {
                        continue;
                    }
                    if let Some(path) = el.attr(attr).filter(|p| !p.is_empty()) {
                        textures.insert(path.to_owned());
                    }
                }
                if let Some(path) = el.attr("font").filter(|p| !p.is_empty()) {
                    fonts.insert(path.to_owned());
                }
            }
            for child in &el.children {
                visit(child, textures, fonts);
            }
        }
        for item in &doc.items {
            match item {
                TopLevel::Font(el) | TopLevel::Template(el) | TopLevel::Instance(el) => {
                    visit(el, &mut self.textures, &mut self.fonts)
                }
                _ => {}
            }
        }
        self.request_art();
    }

    /// Discover the embedded side through its original builtin reader, including
    /// relative includes and its basename fallback. No embedded code is executed.
    #[cfg(any(target_arch = "wasm32", feature = "port-gates"))]
    pub(crate) fn discover_builtin(
        &mut self,
        roots: impl IntoIterator<Item = String>,
    ) -> Result<(), String> {
        let mut pending: Vec<_> = roots.into_iter().map(|path| (path, 0usize)).collect();
        let mut seen = HashSet::new();
        while let Some((path, depth)) = pending.pop() {
            if depth > 64 {
                return Err(format!(
                    "builtin XML include depth exceeds delivery limit: {path}"
                ));
            }
            if !path.to_ascii_lowercase().ends_with(".xml")
                || !seen.insert(benilla_assets::normalize_path(&path))
            {
                continue;
            }
            let bytes = crate::ui_script::addons::read_builtin_file(&path)
                .ok_or_else(|| format!("builtin UI file absent: {path}"))?;
            let doc = benilla_ui::framexml::parse(&benilla_ui::source::decode(&bytes))
                .map_err(|e| format!("{path}: {e}"))?;
            self.discover_art(&doc);
            let normalized = path.replace('\\', "/");
            let base = normalized.rsplit_once('/').map_or("", |(base, _)| base);
            for item in doc.items {
                if let TopLevel::Include(req) = item {
                    pending.push((benilla_ui::loader::join_ref(base, &req), depth + 1));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn presence_ready(&self) -> bool {
        self.presence.is_some()
    }

    pub(crate) fn into_snapshot(
        mut self,
    ) -> Result<(Vec<(String, Vec<u8>)>, benilla_formats::ChainPresenceIndex), String> {
        let index = self
            .presence
            .take()
            .ok_or_else(|| "UI presence index not delivered".to_string())?;
        Ok((self.into_files()?, index))
    }

    pub(crate) fn accept(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        let key = benilla_assets::normalize_path(path);
        if !self.files.contains_key(&key) {
            return Err(format!("unrequested UI file: {path}"));
        }
        // Lua is never decoded or run here. XML discovery uses the production parser,
        // including its encoding policy and top-level Include/Script semantics.
        let mut dependencies = Vec::new();
        let normalized = path.replace('\\', "/");
        let base = normalized.rsplit_once('/').map_or("", |(base, _)| base);
        if path.to_ascii_lowercase().ends_with(".toc") {
            let toc = benilla_ui::toc::Toc::parse(&benilla_ui::source::decode(&bytes));
            // These are installed addon TOCs, not the synthetic FrameXML
            // manifest. LoadAddOn resolves their entries against the addon's
            // folder before its chain reader adds Interface/AddOns/.
            dependencies.extend(
                toc.files
                    .iter()
                    .map(|req| benilla_ui::loader::join_ref(base, req)),
            );
        }
        if path.to_ascii_lowercase().ends_with(".xml") {
            match benilla_ui::framexml::parse(&benilla_ui::source::decode(&bytes)) {
                Ok(doc) => {
                    self.discover_art(&doc);
                    for item in doc.items {
                        let req = match item {
                            TopLevel::Include(req) | TopLevel::Script(ScriptRef::File(req)) => req,
                            _ => continue,
                        };
                        dependencies.push(benilla_ui::loader::join_ref(base, &req));
                    }
                }
                // Preserve malformed addon bytes for the original per-addon
                // loader to diagnose. They must not bring down the stock UI.
                Err(error) if self.addons.contains(path) => {
                    bevy::log::warn!("ui_files: addon XML discovery failed for {path}: {error}; original loader will report it");
                }
                Err(error) => return Err(format!("{path}: {error}")),
            }
        }
        self.files.get_mut(&key).unwrap().1 = Some(bytes);
        for path in dependencies {
            // A broken third-party reference remains absent, so the original
            // loader reports its real not-found error rather than boot waiting
            // forever on an HTTP request for a file the catalog says is missing.
            if self.addons.contains(&normalized) && !self.contains(&path) {
                continue;
            }
            self.request(path);
        }
        // A delivered bad first texture can now admit its second candidate;
        // Lua/widget execution still has not started.
        self.request_art();
        Ok(())
    }

    pub(crate) fn into_files(self) -> Result<Vec<(String, Vec<u8>)>, String> {
        self.files
            .into_values()
            .map(|(path, bytes)| {
                bytes
                    .map(|bytes| (path.clone(), bytes))
                    .ok_or_else(|| format!("UI file not delivered: {path}"))
            })
            .collect()
    }
}

#[cfg(feature = "port-gates")]
pub fn verify_ui_file_delivery() {
    use benilla_formats::{FileSource, MemoryFileSource};
    let toc = benilla_ui::toc::Toc::parse(include_str!("../assets/ui/benilla.toc"));
    let mut builtin = UiFilePlan::default();
    builtin
        .discover_builtin(
            toc.files
                .into_iter()
                .filter(|path| !benilla_interface_files::is_install_path(path)),
        )
        .unwrap();
    assert!(
        !builtin.textures.is_empty(),
        "real builtin XML must contribute art hints"
    );
    // Read-only art discovery uses the production parser, including virtual
    // definitions, nested regions, case-insensitive attrs and state textures.
    let mut art = UiFilePlan::new(["boot.xml".into()]);
    art.accept("boot.xml", br#"<Ui><Frame name="Template" virtual="true"><Layers><Layer><Texture FILE="Interface\Boot"/><Texture file=""/></Layer></Layers><NormalTexture file="Interface\Boot"/><Backdrop bgFile="Interface\Absent"/></Frame><Font name="BootFont" font="Fonts\BOOT.TTF"/><Model file="Ignored.mdx"/><Script file="code.lua"/></Ui>"#.to_vec()).unwrap();
    let present = |path: &str| {
        matches!(
            benilla_assets::normalize_path(path).as_str(),
            "interface\\boot.blp" | "interface\\boot.tga" | "fonts\\boot.ttf"
        )
    };
    let mut next = art.art_candidates(present);
    next.sort();
    assert_eq!(next, ["Fonts\\BOOT.TTF", "interface\\boot.blp"]);
    for path in next {
        art.request(path);
    }
    art.accept("code.lua", vec![]).unwrap();
    art.accept("Fonts\\BOOT.TTF", vec![0xff]).unwrap();
    art.accept("interface\\boot.blp", vec![0xff]).unwrap();
    assert_eq!(art.art_candidates(present), ["interface\\boot.tga"]);
    art.request("interface\\boot.tga".into());
    assert!(
        art.pending().any(|p| p == "interface\\boot.tga"),
        "fallback must arrive before publication"
    );
    let mut tga = vec![0; 18];
    tga[2] = 2;
    tga[12] = 1;
    tga[14] = 1;
    tga[16] = 32;
    tga[17] = 0x28;
    tga.extend_from_slice(&[30, 20, 10, 255]);
    art.accept("interface\\boot.tga", tga.clone()).unwrap();
    assert!(art.art_candidates(present).is_empty());
    let mut first_wins = UiFilePlan::new(["boot.xml".into()]);
    first_wins
        .accept(
            "boot.xml",
            br#"<Ui><Texture file="Interface\Boot"/></Ui>"#.to_vec(),
        )
        .unwrap();
    for path in first_wins.art_candidates(present) {
        first_wins.request(path);
    }
    first_wins.accept("interface\\boot.blp", tga).unwrap();
    assert!(
        first_wins.art_candidates(present).is_empty(),
        "valid first content must not require an unused second file"
    );
    let metadata = b"BCPI\x01\x00\x00\x00\x00\x00\x00\x00";
    let mut missing_index = UiFilePlan::new(["a.lua".into()]);
    missing_index.accept("a.lua", vec![]).unwrap();
    assert!(missing_index.into_snapshot().is_err());
    let mut missing_bytes = UiFilePlan::new(["a.lua".into()]);
    missing_bytes.accept_presence(metadata).unwrap();
    assert!(missing_bytes.into_snapshot().is_err());
    let mut invalid_index = UiFilePlan::new(["a.lua".into()]);
    invalid_index.accept("a.lua", vec![]).unwrap();
    assert!(invalid_index.accept_presence(b"broken").is_err());
    assert!(!invalid_index.presence_ready());
    assert!(invalid_index.into_snapshot().is_err());
    for index_first in [true, false] {
        let mut complete = UiFilePlan::new(["a.lua".into()]);
        if index_first {
            complete.accept_presence(metadata).unwrap();
        }
        complete.accept("a.lua", vec![0xff, 0, 0x80]).unwrap();
        if !index_first {
            complete.accept_presence(metadata).unwrap();
        }
        let (files, presence) = complete.into_snapshot().unwrap();
        assert_eq!(files[0].1, [0xff, 0, 0x80]);
        assert!(
            !presence.contains("a.lua"),
            "presence cannot be inferred from resident bytes"
        );
    }
    let mut font_plan = UiFilePlan::new(
        crate::ui_text::client_font_files()
            .iter()
            .map(|p| (*p).to_owned()),
    );
    assert_eq!(font_plan.pending().count(), 4);
    font_plan
        .accept("Fonts\\FRIZQT__.TTF", vec![0xff, 0, 0x80])
        .unwrap();
    assert_eq!(font_plan.pending().count(), 3);
    assert!(
        font_plan.into_files().is_err(),
        "one font cannot publish the complete source"
    );
    let root = "Interface/FrameXML/Main.xml";
    let mut plan = UiFilePlan::new([root.into()]);
    plan.accept(root, br#"<Ui><Script file="first.lua"/><Include file="parts/child.xml"/><Script file="last.lua"/></Ui>"#.to_vec()).unwrap();
    assert_eq!(plan.pending().count(), 3);
    assert!(UiFilePlan::new(["absent.xml".into()]).into_files().is_err());
    // Delivery order differs from XML execution order deliberately.
    plan.accept(
        "Interface/FrameXML/last.lua",
        b"order = order .. 'last'".to_vec(),
    )
    .unwrap();
    plan.accept(
        "Interface/FrameXML/parts/child.xml",
        br#"<Ui><Script file="../child.lua"/></Ui>"#.to_vec(),
    )
    .unwrap();
    plan.accept(
        "Interface/FrameXML/child.lua",
        b"order = order .. 'child,'".to_vec(),
    )
    .unwrap();
    plan.accept("Interface/FrameXML/first.lua", b"order = 'first,'".to_vec())
        .unwrap();
    assert_eq!(plan.pending().count(), 0);
    let source = std::cell::RefCell::new(MemoryFileSource::new(plan.into_files().unwrap()));
    let provider = |path: &str| source.borrow_mut().read_file(path).ok();
    let document =
        benilla_ui::framexml::parse(&benilla_ui::source::decode(&provider(root).unwrap())).unwrap();
    let mut script = benilla_ui::script::UiScript::new().unwrap();
    let report = benilla_ui::loader::load_in(&mut script, &document, root, &provider);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report.missing_files.is_empty(),
        "{:?}",
        report.missing_files
    );
    script.run("assert(order == 'first,child,last')").unwrap();
    // Discovery is byte-transparent for Lua, and bad XML cannot complete publication.
    let mut bytes = UiFilePlan::new(["locale.lua".into()]);
    bytes.accept("locale.lua", vec![0xff, 0x80, 0]).unwrap();
    assert_eq!(bytes.into_files().unwrap()[0].1, [0xff, 0x80, 0]);
    let mut invalid = UiFilePlan::new(["bad.xml".into()]);
    assert!(invalid.accept("bad.xml", b"<Ui>".to_vec()).is_err());
    assert!(invalid.into_files().is_err());
}
