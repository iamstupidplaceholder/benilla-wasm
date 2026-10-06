//! Browser FrameXML/Lua vertical slice over the production host.
//!
//! This deliberately supports one dialog Frame and its first Button, not the
//! full `UiScript` surface — but since A0 the frames, scripts and clicks below
//! are the production ones: `install_dialog` discovers the instance/button
//! names from the XML and materializes the whole document through the
//! production loader (`load_in`), so `OnClick`/`Show`/`Hide`/visibility all run
//! the same code as the native client. The only thing this layer adds is the
//! narrow probe doors (a click, a show/hide flip, a visibility read) keyed by
//! the installed handles.

use std::collections::HashMap;

use crate::{
    framexml::{self, Element, TopLevel},
    loader,
    script::UiScript,
    widget::FrameHandle,
};

pub struct WebUiScript {
    ui: UiScript,
    /// Handle → global frame name, for the installed dialog's windows. The probe
    /// passes handles back opaquely; every door below resolves the name and runs
    /// the production path (a Lua `:Click`/`:Show`/`:Hide`, a host visibility
    /// read), so nothing here forges or caches widget state.
    names: HashMap<FrameHandle, String>,
}

pub struct WebDialog {
    pub frame: FrameHandle,
    pub button: FrameHandle,
    pub button_name: String,
    pub label: String,
}

impl WebUiScript {
    pub fn new() -> Result<Self, String> {
        UiScript::new()
            .map(|ui| Self {
                ui,
                names: HashMap::new(),
            })
            .map_err(|error| error.to_string())
    }

    pub fn exec_lua(&mut self, source: &[u8], name: &str) -> Result<(), String> {
        self.ui
            .run_chunk_named(source, name)
            .map_err(|error| error.to_string())
    }

    /// Read a string global through the production host.
    pub fn global_string(&mut self, name: &str) -> Option<String> {
        self.ui.global_string(name)
    }

    /// Run a chunk with one temporary string global, restoring its previous string/nil value.
    ///
    /// `chunk_name` is kept for the probe's call shape; the production host runs
    /// this door under its own fixed chunk name, so the value only documents intent.
    pub fn exec_with_string_global(
        &mut self,
        name: &str,
        value: &str,
        source: &[u8],
        chunk_name: &str,
    ) -> Result<(), String> {
        let _ = chunk_name;
        self.ui
            .run_with_string_global(name, value, &String::from_utf8_lossy(source))
            .map_err(|error| error.to_string())
    }

    /// Expand the named instance and bind its first Button's shipped OnClick.
    ///
    /// Discovery (which element is the instance, which descendant is the first
    /// Button, what `$parent` expands to, which global names the label) is a
    /// pure XML walk with the same errors as before; materialization is the
    /// production loader over the whole document, so the dialog's frames wear
    /// the real wrappers and its scripts are the registered `OnClick`s. A
    /// loader report with `errors` fails the install — warnings and
    /// missing-file rows do not, exactly like the reference's non-fatal legs.
    pub fn install_dialog(&mut self, xml: &str, instance_name: &str) -> Result<WebDialog, String> {
        let document = framexml::parse(xml).map_err(|error| error.to_string())?;
        let templates = document.templates();
        let instance = document
            .items
            .iter()
            .find_map(|item| match item {
                TopLevel::Instance(element)
                    if element.tag.eq_ignore_ascii_case("Frame")
                        && element.name() == Some(instance_name) =>
                {
                    Some(element)
                }
                _ => None,
            })
            .ok_or_else(|| format!("FrameXML has no frame named {instance_name}"))?;
        let mut warnings = Vec::new();
        let expanded = framexml::expand(instance, &templates, &mut warnings);
        let button = descendant(&expanded, "Button")
            .ok_or_else(|| format!("{instance_name} has no Button"))?;
        let raw_name = button
            .name()
            .ok_or_else(|| format!("{instance_name} Button is unnamed"))?;
        let button_name = framexml::resolve_name(raw_name, instance_name);
        let label_key = child(button, "ButtonText")
            .and_then(|element| element.attr("text"))
            .ok_or_else(|| format!("{button_name} has no ButtonText token"))?;

        // The document's own path is what relative `<Include>`/`<Script file=>`
        // resolve against; inline probe documents carry none, so there is no
        // path to give and no provider to read from. Missing-file rows are
        // reported, never fatal (decision 2155).
        let no_files = |_: &str| -> Option<Vec<u8>> { None };
        let report = loader::load_in(&mut self.ui, &document, "", &no_files);
        if !report.errors.is_empty() {
            return Err(format!(
                "FrameXML errors installing {instance_name}: {}",
                report.errors.join("; ")
            ));
        }
        let frame = self.ui.frame_handle(instance_name).ok_or_else(|| {
            format!("FrameXML has no frame named {instance_name} after load")
        })?;
        let button_handle = self.ui.frame_handle(&button_name).ok_or_else(|| {
            format!("{instance_name} has no Button {button_name} after load")
        })?;
        // The label is the global the loader resolved the ButtonText through,
        // read back after the load so a missing token fails here rather than
        // surfacing as an empty caption downstream.
        let label: String = self
            .ui
            .global_string(label_key)
            .ok_or_else(|| format!("read {label_key}: no such string"))?;
        self.names.insert(frame, instance_name.to_owned());
        self.names.insert(button_handle, button_name.clone());
        Ok(WebDialog {
            frame,
            button: button_handle,
            button_name,
            label,
        })
    }

    /// Run the button's shipped `OnClick` through the production click path
    /// (`Button:Click` → the enabled gate → `OnClick(self, button, down)`).
    pub fn click(&mut self, button: FrameHandle) -> Result<(), String> {
        let name = self
            .names
            .get(&button)
            .cloned()
            .ok_or_else(|| "button handle is not an installed dialog button".to_owned())?;
        self.ui
            .run(&format!("{name}:Click(\"LeftButton\")"))
            .map_err(|error| error.to_string())
    }

    pub fn set_shown(&mut self, frame: FrameHandle, shown: bool) -> Result<(), String> {
        let name = self
            .names
            .get(&frame)
            .cloned()
            .ok_or_else(|| "frame handle is not an installed dialog window".to_owned())?;
        self.ui
            .run(&format!("{name}:{}()", if shown { "Show" } else { "Hide" }))
            .map_err(|error| error.to_string())
    }

    pub fn visible(&self, frame: FrameHandle) -> Result<bool, String> {
        let name = self
            .names
            .get(&frame)
            .ok_or_else(|| "frame handle is not an installed dialog window".to_owned())?;
        Ok(self.ui.frame_visible(name))
    }
}

fn child<'a>(element: &'a Element, tag: &str) -> Option<&'a Element> {
    element
        .children
        .iter()
        .find(|child| child.tag.eq_ignore_ascii_case(tag))
}

fn descendant<'a>(element: &'a Element, tag: &str) -> Option<&'a Element> {
    element.children.iter().find_map(|child| {
        if child.tag.eq_ignore_ascii_case(tag) {
            Some(child)
        } else {
            descendant(child, tag)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::WebUiScript;

    const XML: &str = r#"
        <Ui>
          <Frame name="DialogBoxFrame" virtual="true">
            <Frames><Button name="$parentButton">
              <ButtonText text="OKAY"/>
              <Scripts><OnClick>this:GetParent():Hide()</OnClick></Scripts>
            </Button></Frames>
          </Frame>
          <Frame name="ScriptErrors" inherits="DialogBoxFrame"/>
        </Ui>
    "#;

    #[test]
    fn shipped_style_click_hides_production_frame() {
        let mut ui = WebUiScript::new().expect("web UI");
        ui.exec_lua(b"OKAY = 'Okay'", "@Interface/FrameXML/GlobalStrings.lua")
            .expect("strings");
        let dialog = ui.install_dialog(XML, "ScriptErrors").expect("dialog");
        assert_eq!(dialog.button_name, "ScriptErrorsButton");
        assert_eq!(dialog.label, "Okay");
        assert!(ui.visible(dialog.frame).expect("visible"));
        ui.click(dialog.button).expect("OnClick");
        assert!(!ui.visible(dialog.frame).expect("hidden"));
        ui.set_shown(dialog.frame, true).expect("show");
        assert!(ui.visible(dialog.frame).expect("visible again"));
    }
}
