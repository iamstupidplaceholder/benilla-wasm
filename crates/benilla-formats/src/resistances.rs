//! Original vanilla Resistances.dbc school vocabulary (12 fields, name at 3).
use crate::{
    dbc::{parse, str_at, u32_at},
    FileSource,
};
use anyhow::{Context, Result};
use benilla_dbc::{FieldType, Schema, SchemaField};
use std::collections::HashMap;

pub struct ResistanceCatalog {
    pub names: HashMap<u8, String>,
}

pub fn load_resistance_catalog(source: &mut impl FileSource) -> Result<ResistanceCatalog> {
    let bytes = source
        .read_file("DBFilesClient/Resistances.dbc")
        .context("reading Resistances.dbc")?;
    let mut schema = Schema::new("Resistances");
    for field in 0..12 {
        schema.add_field(SchemaField::new(
            format!("F{field}"),
            if field == 3 {
                FieldType::String
            } else {
                FieldType::UInt32
            },
        ));
    }
    let rows = parse(&bytes, schema, "Resistances.dbc")?;
    let mut names = HashMap::new();
    for row in rows.records() {
        if let (Some(id), Some(name)) = (u32_at(row, 0), str_at(&rows, row, 3)) {
            if id < 7 && !name.is_empty() {
                names.insert(id as u8, name);
            }
        }
    }
    anyhow::ensure!(
        names.len() == 7,
        "Resistances.dbc lacks one or more school names"
    );
    Ok(ResistanceCatalog { names })
}
