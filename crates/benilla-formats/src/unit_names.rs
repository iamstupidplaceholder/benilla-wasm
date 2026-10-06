//! Original localized unit vocabulary from ChrRaces / CreatureType DBCs.
use crate::{
    dbc::{parse, str_at, u32_at},
    FileSource,
};
use anyhow::{Context, Result};
use benilla_dbc::{FieldType, Schema, SchemaField};
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct UnitNames {
    races: HashMap<u32, String>,
    creatures: HashMap<u32, String>,
}

impl UnitNames {
    pub fn race(&self, id: u32) -> Option<&str> {
        self.races.get(&id).map(String::as_str)
    }
    pub fn creature(&self, id: u32) -> Option<&str> {
        self.creatures.get(&id).map(String::as_str)
    }
}

fn names(
    source: &mut impl FileSource,
    table: &str,
    fields: usize,
    first: usize,
) -> Result<HashMap<u32, String>> {
    let path = format!("DBFilesClient\\{table}.dbc");
    let bytes = source
        .read_file(&path)
        .with_context(|| format!("reading {path}"))?;
    let mut schema = Schema::new(table);
    for field in 0..fields {
        schema.add_field(SchemaField::new(
            format!("f{field}"),
            if (first..first + 8).contains(&field) {
                FieldType::String
            } else {
                FieldType::UInt32
            },
        ));
    }
    let rows = parse(&bytes, schema, table)?;
    let mut result = HashMap::new();
    for row in rows.records() {
        if let (Some(id), Some(name)) = (u32_at(row, 0), str_at(&rows, row, first)) {
            result.insert(id, name);
        }
    }
    Ok(result)
}

pub fn load_unit_names(source: &mut impl FileSource) -> Result<UnitNames> {
    Ok(UnitNames {
        races: names(source, "ChrRaces", 29, 17)?,
        creatures: names(source, "CreatureType", 11, 1)?,
    })
}
