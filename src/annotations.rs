//! Reading the `@sql_*` schema annotations.
//!
//! Annotations live on the schema fragment and do not survive into the
//! validator schema, so the schema text is parsed again as a fragment (the
//! pattern of `cedar-policy-symcc`'s `@semantics` collector). Entity types
//! carry `@sql_table`, `@sql_tags_table`, `@sql_primary_key`,
//! `@sql_entity_id_column` and `@sql_custom_config`; attributes carry
//! `@sql_column`, `@sql_unique` and `@sql_custom_attrs`. Attribute annotations
//! are read from the entity type's record shape, or from the common type the
//! shape names.

use std::collections::HashMap;
use std::str::FromStr;

use cedar_policy_core::ast::EntityType;
use cedar_policy_core::est::Annotations;
use cedar_policy_core::extensions::Extensions;
use cedar_policy_core::validator::RawName;
use cedar_policy_core::validator::json_schema::{
    EntityTypeKind, Fragment, NamespaceDefinition, RecordType, Type, TypeVariant,
};
use smol_str::SmolStr;

use crate::config::CustomConfig;
use crate::ident::SQLIdentifier;
use crate::{Error, Result};

/// `@sql_table("users")`: the table name.
pub const TABLE: &str = "sql_table";
/// `@sql_tags_table("users_tags")`: the tags table name.
pub const TAGS_TABLE: &str = "sql_tags_table";
/// `@sql_primary_key("id")`: the primary key column.
pub const PRIMARY_KEY: &str = "sql_primary_key";
/// `@sql_entity_id_column("id")`: the column holding the entity id.
pub const ENTITY_ID_COLUMN: &str = "sql_entity_id_column";
/// `@sql_custom_config("{ … }")`: custom columns and table modifiers as JSON.
pub const CUSTOM_CONFIG: &str = "sql_custom_config";
/// `@sql_column("first_name")`: the column name of an attribute.
pub const COLUMN: &str = "sql_column";
/// `@sql_unique("true")`: whether the attribute's column is unique.
pub const UNIQUE: &str = "sql_unique";
/// `@sql_custom_attrs("DEFAULT TRUE")`: raw SQL appended to the column.
pub const CUSTOM_ATTRS: &str = "sql_custom_attrs";

/// The `@sql_*` annotations of a schema.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SqlAnnotations {
    /// By fully-qualified entity type.
    pub entity_types: HashMap<EntityType, EntityTypeAnnotations>,
}

/// The annotations of one entity type and its attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityTypeAnnotations {
    /// `@sql_table`.
    pub table: Option<SQLIdentifier>,
    /// `@sql_tags_table`.
    pub tags_table: Option<SQLIdentifier>,
    /// `@sql_primary_key`.
    pub primary_key: Option<SQLIdentifier>,
    /// `@sql_entity_id_column`.
    pub entity_id_column: Option<SQLIdentifier>,
    /// `@sql_custom_config`.
    pub custom_config: Option<CustomConfig>,
    /// The attributes' annotations, by attribute name.
    pub attributes: HashMap<SmolStr, AttributeAnnotations>,
}

/// The annotations of one attribute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttributeAnnotations {
    /// `@sql_column`.
    pub column: Option<SQLIdentifier>,
    /// `@sql_unique`.
    pub unique: bool,
    /// `@sql_custom_attrs`.
    pub custom_attrs: Option<String>,
}

/// Collects the annotations of a schema in the Cedar schema syntax.
pub fn from_cedarschema_str(src: &str) -> Result<SqlAnnotations> {
    let (fragment, _warnings) =
        Fragment::<RawName>::from_cedarschema_str(src, Extensions::all_available())
            .map_err(|e| Error::Schema(e.to_string()))?;
    collect(&fragment)
}

/// Collects the annotations of a schema in the JSON syntax.
pub fn from_json_str(src: &str) -> Result<SqlAnnotations> {
    let fragment =
        Fragment::<RawName>::from_json_str(src).map_err(|e| Error::Schema(e.to_string()))?;
    collect(&fragment)
}

/// Collects the annotations of a parsed fragment.
pub fn collect(fragment: &Fragment<RawName>) -> Result<SqlAnnotations> {
    let mut out = SqlAnnotations::default();
    for (namespace, def) in &fragment.0 {
        for (id, entity_type) in &def.entity_types {
            let name = match namespace {
                Some(ns) => format!("{ns}::{id}"),
                None => id.to_string(),
            };
            let ety = EntityType::from_str(&name).map_err(|e| {
                Error::Schema(format!("the entity type name {name} does not parse: {e}"))
            })?;
            let on = format!("entity type {ety}");
            let a = &entity_type.annotations;
            let mut ann = EntityTypeAnnotations {
                table: identifier(a, TABLE, &on)?,
                tags_table: identifier(a, TAGS_TABLE, &on)?,
                primary_key: identifier(a, PRIMARY_KEY, &on)?,
                entity_id_column: identifier(a, ENTITY_ID_COLUMN, &on)?,
                custom_config: None,
                attributes: HashMap::new(),
            };
            if let Some(json) = value(a, CUSTOM_CONFIG) {
                ann.custom_config =
                    Some(serde_json::from_str(json).map_err(|e| Error::Annotation {
                        annotation: CUSTOM_CONFIG,
                        on: on.clone(),
                        message: e.to_string(),
                    })?);
            }
            if let EntityTypeKind::Standard(standard) = &entity_type.kind
                && let Some(record) = resolve_record(&standard.shape.0, def, fragment)
            {
                for (attr, attr_ty) in &record.attributes {
                    let on = format!("attribute {attr} of {ety}");
                    let a = &attr_ty.annotations;
                    let attr_ann = AttributeAnnotations {
                        column: identifier(a, COLUMN, &on)?,
                        unique: boolean(a, UNIQUE, &on)?,
                        custom_attrs: value(a, CUSTOM_ATTRS).map(str::to_owned),
                    };
                    if attr_ann != AttributeAnnotations::default() {
                        ann.attributes.insert(attr.clone(), attr_ann);
                    }
                }
            }
            if ann != EntityTypeAnnotations::default() {
                out.entity_types.insert(ety, ann);
            }
        }
    }
    Ok(out)
}

/// The record type an entity shape denotes: the record itself, or the record
/// behind a common type name (same namespace first, then the empty one).
fn resolve_record<'f>(
    ty: &'f Type<RawName>,
    def: &'f NamespaceDefinition<RawName>,
    fragment: &'f Fragment<RawName>,
) -> Option<&'f RecordType<RawName>> {
    match ty {
        Type::Type {
            ty: TypeVariant::Record(record),
            ..
        } => Some(record),
        Type::CommonTypeRef { type_name, .. }
        | Type::Type {
            ty: TypeVariant::EntityOrCommon { type_name },
            ..
        } => {
            let wanted = type_name.to_string();
            let empty = fragment.0.get(&None);
            [Some(def), empty]
                .into_iter()
                .flatten()
                .flat_map(|d| d.common_types.iter())
                .find(|(id, _)| id.to_string() == wanted)
                .and_then(|(_, common)| resolve_record(&common.ty, def, fragment))
        }
        Type::Type { .. } => None,
    }
}

/// The annotation's value; a bare `@key` (no value) counts as the empty string.
fn value<'a>(annotations: &'a Annotations, key: &str) -> Option<&'a str> {
    annotations
        .0
        .iter()
        .find(|(id, _)| id.as_ref() == key)
        .map(|(_, a)| a.as_ref().map_or("", |a| a.val.as_str()))
}

fn identifier(
    annotations: &Annotations,
    key: &'static str,
    on: &str,
) -> Result<Option<SQLIdentifier>> {
    value(annotations, key)
        .map(|v| {
            SQLIdentifier::new(v).map_err(|e| Error::Annotation {
                annotation: key,
                on: on.to_owned(),
                message: e.to_string(),
            })
        })
        .transpose()
}

fn boolean(annotations: &Annotations, key: &'static str, on: &str) -> Result<bool> {
    match value(annotations, key) {
        None => Ok(false),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(Error::Annotation {
            annotation: key,
            on: on.to_owned(),
            message: format!("expected \"true\" or \"false\", got {other:?}"),
        }),
    }
}
