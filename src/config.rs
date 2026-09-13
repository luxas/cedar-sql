//! The database, table and column model a Cedar schema maps to.
//!
//! [`DatabaseConfiguration::from_schema`] applies the defaults of the README
//! (one table per entity type named after it, one column per attribute, a
//! `<table>_tags` table per entity type with tags, one hierarchy table, and the
//! `__entity_id`/`__entity_type` interface columns every entity table
//! exposes), and [`DatabaseConfiguration::from_schema_with_annotations`]
//! applies the `@sql_*` annotations collected by [`crate::annotations`].

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cedar_policy::Schema;
use cedar_policy_core::ast::EntityType;
use cedar_policy_core::validator::types::{EntityKind, OpenTag, Type};
use cedar_policy_core::validator::{ValidatorEntityType, ValidatorSchema};
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::annotations::{EntityTypeAnnotations, SqlAnnotations};
use crate::ident::SQLIdentifier;
use crate::{Error, Result};

/// The default name of the interface column holding the entity id.
pub const DEFAULT_ENTITY_ID_COLUMN: &str = "__entity_id";
/// The default name of the interface column holding the entity type.
pub const DEFAULT_ENTITY_TYPE_COLUMN: &str = "__entity_type";
/// The default name of the entity hierarchy table.
pub const DEFAULT_HIERARCHY_TABLE: &str = "cedar_entity_hierarchy";
/// The suffix of a default tags table name.
pub const TAGS_TABLE_SUFFIX: &str = "_tags";
/// The tags table column holding the tagged entity's id.
pub const TAGS_ENTITY_ID_COLUMN: &str = "entity_id";
/// The tags table column holding the tag.
pub const TAGS_TAG_COLUMN: &str = "tag";
/// The tags table column holding the tag's value.
pub const TAGS_VALUE_COLUMN: &str = "value";
/// The hierarchy table columns, in order.
pub const HIERARCHY_COLUMNS: [&str; 4] = [
    "descendant_type",
    "descendant_id",
    "ancestor_type",
    "ancestor_id",
];

/// A column type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SQLType {
    /// Cedar `String`, and entity references (the referenced entity's id).
    Text,
    /// Cedar `Long`.
    BigInt,
    /// Cedar `Bool`.
    Bool,
    /// Cedar `Set` and `Record`, in the canonical JSON encoding of
    /// [`crate::load::canonical_json`].
    Jsonb,
    /// An array column; not used by the Cedar mapping (sets are [`SQLType::Jsonb`]).
    Set(Box<SQLType>),
    /// A type this crate does not interpret, for custom columns.
    Custom(String),
}

impl SQLType {
    /// The column type for a Cedar attribute (or tag) type, and the entity
    /// type the column references when it is an entity reference.
    pub fn for_cedar_type(ty: &Type) -> Result<(SQLType, Option<EntityType>)> {
        Ok(match ty {
            Type::Bool(_) => (SQLType::Bool, None),
            Type::Long => (SQLType::BigInt, None),
            Type::String => (SQLType::Text, None),
            Type::Entity(EntityKind::Entity(lub)) => match lub.get_single_entity() {
                Some(ety) => (SQLType::Text, Some(ety.clone())),
                None => return Err(Error::Unsupported("attributes of a union entity type")),
            },
            Type::Entity(EntityKind::AnyEntity) => {
                return Err(Error::Unsupported(
                    "attributes of an unspecified entity type",
                ));
            }
            Type::Set { .. } | Type::Record { .. } => (SQLType::Jsonb, None),
            Type::ExtensionType { .. } => return Err(Error::Unsupported("extension types")),
            Type::Never => return Err(Error::Unsupported("attributes of the never type")),
        })
    }
}

/// A foreign key target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignKey {
    /// The referenced table.
    pub table: SQLIdentifier,
    /// The referenced column (the table's entity id column).
    pub column: SQLIdentifier,
    /// The referenced entity type.
    pub entity_type: EntityType,
}

/// One column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnConfiguration {
    /// The column type.
    pub ty: SQLType,
    /// Whether the column may be `NULL` (an optional Cedar attribute).
    pub nullable: bool,
    /// Whether the column carries a `UNIQUE` constraint.
    pub unique: bool,
    /// Raw SQL appended to the column definition (`@sql_custom_attrs`).
    pub custom_attrs: Option<String>,
    /// The foreign key the column carries, if it is an entity reference.
    pub references: Option<ForeignKey>,
    /// The expression of a stored generated column.
    pub generated: Option<String>,
}

impl ColumnConfiguration {
    fn new(ty: SQLType) -> Self {
        Self {
            ty,
            nullable: false,
            unique: false,
            custom_attrs: None,
            references: None,
            generated: None,
        }
    }
}

/// The tags table of an entity type with tags: columns
/// [`TAGS_ENTITY_ID_COLUMN`], [`TAGS_TAG_COLUMN`] and [`TAGS_VALUE_COLUMN`],
/// with the first two as the primary key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagsTableConfiguration {
    /// The table name.
    pub table: SQLIdentifier,
    /// The value column.
    pub value: ColumnConfiguration,
}

/// One entity table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableConfiguration {
    /// The entity type the table stores.
    pub entity_type: EntityType,
    /// The primary key columns.
    pub primary_keys: IndexSet<SQLIdentifier>,
    /// The columns, in definition order.
    pub columns: IndexMap<SQLIdentifier, ColumnConfiguration>,
    /// The tags table, when the entity type has tags.
    pub tags: Option<TagsTableConfiguration>,
    /// The column holding the entity id, which entity lookups join on. It is
    /// the interface column itself unless `@sql_entity_id_column` (or the
    /// custom configuration) named another column, in which case the
    /// interface column is generated from it.
    pub entity_id_column: SQLIdentifier,
    /// Which column each Cedar attribute is stored in.
    pub attribute_columns: IndexMap<SmolStr, SQLIdentifier>,
}

/// The `@sql_custom_config` JSON: custom columns and table modifiers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomConfig {
    /// Must be absent or `null`; the entity type comes from the schema.
    #[serde(default)]
    pub entity_type: Option<()>,
    /// The primary key columns, when not the entity id column.
    #[serde(default)]
    pub primary_keys: Vec<SQLIdentifier>,
    /// The column holding the entity id.
    #[serde(default)]
    pub entity_id_column: Option<SQLIdentifier>,
    /// Additional columns.
    #[serde(default)]
    pub columns: IndexMap<SQLIdentifier, CustomColumn>,
}

/// One custom column of a [`CustomConfig`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomColumn {
    /// The column type.
    pub ty: SQLType,
    /// Whether the column may be `NULL`.
    #[serde(default)]
    pub nullable: bool,
    /// Whether the column carries a `UNIQUE` constraint.
    #[serde(default)]
    pub unique: bool,
    /// Raw SQL appended to the column definition.
    #[serde(default)]
    pub custom_attrs: Option<String>,
}

/// The whole database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseConfiguration {
    /// The entity tables by name, in entity type order.
    pub tables: IndexMap<SQLIdentifier, TableConfiguration>,
    /// The hierarchy table: [`HIERARCHY_COLUMNS`], all forming the primary key.
    pub entity_hierarchy_table: SQLIdentifier,
    /// The interface column every entity table exposes for the entity id.
    pub entity_id_column: SQLIdentifier,
    /// The interface column every entity table exposes for the entity type.
    pub entity_type_column: SQLIdentifier,
    /// Whether the DDL declares the foreign keys. Cedar data may reference
    /// entities that do not exist, so the differential tests turn this off.
    pub emit_foreign_keys: bool,
    /// Whether the hierarchy table holds the transitive closure, in which
    /// case `in` needs no recursion.
    pub hierarchy_closed: bool,
    entity_tables: HashMap<EntityType, SQLIdentifier>,
}

impl DatabaseConfiguration {
    /// The default mapping of `schema`, without annotations.
    pub fn from_schema(schema: &Schema) -> Result<Self> {
        Self::from_schema_with_annotations(schema, &SqlAnnotations::default())
    }

    /// The mapping of `schema` under its `@sql_*` annotations.
    pub fn from_schema_with_annotations(
        schema: &Schema,
        annotations: &SqlAnnotations,
    ) -> Result<Self> {
        Builder::new(schema.as_ref(), annotations).build()
    }

    /// Parses a schema in the Cedar schema syntax, collecting its annotations.
    pub fn from_cedarschema_str(src: &str) -> Result<(Self, Schema)> {
        let (schema, _warnings) =
            Schema::from_cedarschema_str(src).map_err(|e| Error::Schema(e.to_string()))?;
        let annotations = crate::annotations::from_cedarschema_str(src)?;
        Ok((
            Self::from_schema_with_annotations(&schema, &annotations)?,
            schema,
        ))
    }

    /// Parses a schema in the JSON syntax, collecting its annotations.
    pub fn from_json_str(src: &str) -> Result<(Self, Schema)> {
        let schema = Schema::from_json_str(src).map_err(|e| Error::Schema(e.to_string()))?;
        let annotations = crate::annotations::from_json_str(src)?;
        Ok((
            Self::from_schema_with_annotations(&schema, &annotations)?,
            schema,
        ))
    }

    /// The table storing `ety`, if any (action types have none).
    pub fn table_for(&self, ety: &EntityType) -> Option<(&SQLIdentifier, &TableConfiguration)> {
        let name = self.entity_tables.get(ety)?;
        Some((name, self.tables.get(name)?))
    }

    /// The column storing attribute `attr` of `ety`.
    pub fn column_for(&self, ety: &EntityType, attr: &str) -> Option<&SQLIdentifier> {
        self.table_for(ety)?.1.attribute_columns.get(attr)
    }
}

/// A table under construction.
struct Draft {
    name: SQLIdentifier,
    entity_type: EntityType,
    columns: IndexMap<SQLIdentifier, ColumnConfiguration>,
    attribute_columns: IndexMap<SmolStr, SQLIdentifier>,
    /// Entity references, resolved to foreign keys once every table is named.
    references: Vec<(SQLIdentifier, EntityType)>,
    entity_id_target: Option<SQLIdentifier>,
    primary_keys: Vec<SQLIdentifier>,
    tags: Option<(SQLIdentifier, SQLType, Option<EntityType>)>,
}

struct Builder<'a> {
    schema: &'a ValidatorSchema,
    annotations: &'a SqlAnnotations,
}

impl<'a> Builder<'a> {
    fn new(schema: &'a ValidatorSchema, annotations: &'a SqlAnnotations) -> Self {
        Self {
            schema,
            annotations,
        }
    }

    fn build(self) -> Result<DatabaseConfiguration> {
        let mut types: Vec<&ValidatorEntityType> = self.schema.entity_types().collect();
        types.sort_by_key(|t| t.name().to_string());
        let empty = EntityTypeAnnotations::default();
        let drafts = types
            .iter()
            .map(|vet| {
                let ann = self
                    .annotations
                    .entity_types
                    .get(vet.name())
                    .unwrap_or(&empty);
                self.draft(vet, ann)
            })
            .collect::<Result<Vec<Draft>>>()?;

        // Table names must be distinct, tags tables included.
        let mut table_names: BTreeMap<&SQLIdentifier, String> = BTreeMap::new();
        for draft in &drafts {
            let owner = format!("entity type {}", draft.entity_type);
            if let Some(previous) = table_names.insert(&draft.name, owner.clone()) {
                return Err(Error::Schema(format!(
                    "table {} would store both {previous} and {owner}; use @sql_table",
                    draft.name
                )));
            }
        }
        for draft in &drafts {
            if let Some((tags, _, _)) = &draft.tags {
                let owner = format!("the tags of entity type {}", draft.entity_type);
                if let Some(previous) = table_names.insert(tags, owner.clone()) {
                    return Err(Error::Schema(format!(
                        "table {tags} would store both {previous} and {owner}; use @sql_tags_table"
                    )));
                }
            }
        }
        let entity_tables: HashMap<EntityType, SQLIdentifier> = drafts
            .iter()
            .map(|d| (d.entity_type.clone(), d.name.clone()))
            .collect();

        // The interface columns and the hierarchy table get globally unique
        // names: the default, or the default with the first free suffix >= 2.
        let column_names: BTreeSet<&str> = drafts
            .iter()
            .flat_map(|d| d.columns.keys().map(SQLIdentifier::as_str))
            .collect();
        let entity_id_column = unique_name(DEFAULT_ENTITY_ID_COLUMN, &column_names)?;
        let entity_type_column = unique_name(DEFAULT_ENTITY_TYPE_COLUMN, &column_names)?;
        let taken_tables: BTreeSet<&str> = table_names.keys().map(|n| n.as_str()).collect();
        let entity_hierarchy_table = unique_name(DEFAULT_HIERARCHY_TABLE, &taken_tables)?;

        let mut tables = IndexMap::new();
        for draft in drafts {
            let table = self.finish(
                draft,
                &entity_id_column,
                &entity_type_column,
                &entity_tables,
            )?;
            tables.insert(table.0, table.1);
        }
        // Foreign keys to tables finished later than the referencing table.
        let targets: HashMap<SQLIdentifier, SQLIdentifier> = tables
            .iter()
            .map(|(name, t)| (name.clone(), t.entity_id_column.clone()))
            .collect();
        for table in tables.values_mut() {
            for column in table.columns.values_mut() {
                if let Some(fk) = &mut column.references {
                    fk.column = targets[&fk.table].clone();
                }
            }
            if let Some(tags) = &mut table.tags
                && let Some(fk) = &mut tags.value.references
            {
                fk.column = targets[&fk.table].clone();
            }
        }
        Ok(DatabaseConfiguration {
            tables,
            entity_hierarchy_table,
            entity_id_column,
            entity_type_column,
            emit_foreign_keys: true,
            hierarchy_closed: false,
            entity_tables,
        })
    }

    fn draft(&self, vet: &ValidatorEntityType, ann: &EntityTypeAnnotations) -> Result<Draft> {
        let ety = vet.name().clone();
        let name = match &ann.table {
            Some(name) => name.clone(),
            None => SQLIdentifier::new(ety.to_string()).map_err(|_| {
                Error::Schema(format!(
                    "the entity type name {ety} is not a valid table name; use @sql_table"
                ))
            })?,
        };
        if vet.open_attributes() == OpenTag::OpenAttributes {
            return Err(Error::Unsupported(
                "entity types with additional attributes",
            ));
        }
        let mut columns: IndexMap<SQLIdentifier, ColumnConfiguration> = IndexMap::new();
        let mut attribute_columns = IndexMap::new();
        let mut references = Vec::new();
        for (attr, attr_ty) in vet.attributes().iter() {
            let attr_ann = ann.attributes.get(attr);
            let column = match attr_ann.and_then(|a| a.column.clone()) {
                Some(column) => column,
                None => SQLIdentifier::new(attr.as_str()).map_err(|_| {
                    Error::Schema(format!(
                        "attribute {attr} of {ety} is not a valid column name; use @sql_column"
                    ))
                })?,
            };
            if let Some(other) = attribute_columns
                .iter()
                .find(|(_, c): &(&SmolStr, &SQLIdentifier)| **c == column)
            {
                return Err(Error::Schema(format!(
                    "attributes {} and {attr} of {ety} both map to column {column}",
                    other.0
                )));
            }
            let (ty, reference) = SQLType::for_cedar_type(&attr_ty.attr_type)?;
            let mut config = ColumnConfiguration::new(ty);
            config.nullable = !attr_ty.is_required;
            if let Some(a) = attr_ann {
                config.unique = a.unique;
                config.custom_attrs = a.custom_attrs.clone();
            }
            if let Some(target) = reference {
                references.push((column.clone(), target));
            }
            columns.insert(column.clone(), config);
            attribute_columns.insert(attr.clone(), column);
        }
        let mut entity_id_target = ann.entity_id_column.clone();
        let mut primary_keys: Vec<SQLIdentifier> = ann.primary_key.iter().cloned().collect();
        if let Some(custom) = &ann.custom_config {
            for (column, custom_column) in &custom.columns {
                if columns.contains_key(column) {
                    return Err(Error::Schema(format!(
                        "custom column {column} of {ety} collides with an attribute column"
                    )));
                }
                columns.insert(
                    column.clone(),
                    ColumnConfiguration {
                        ty: custom_column.ty.clone(),
                        nullable: custom_column.nullable,
                        unique: custom_column.unique,
                        custom_attrs: custom_column.custom_attrs.clone(),
                        references: None,
                        generated: None,
                    },
                );
            }
            match (&entity_id_target, &custom.entity_id_column) {
                (Some(a), Some(b)) if a != b => {
                    return Err(Error::Schema(format!(
                        "@sql_entity_id_column and @sql_custom_config of {ety} name different entity id columns"
                    )));
                }
                (None, Some(b)) => entity_id_target = Some(b.clone()),
                _ => {}
            }
            if !primary_keys.is_empty()
                && !custom.primary_keys.is_empty()
                && primary_keys != custom.primary_keys
            {
                return Err(Error::Schema(format!(
                    "@sql_primary_key and @sql_custom_config of {ety} name different primary keys"
                )));
            }
            if primary_keys.is_empty() {
                primary_keys = custom.primary_keys.clone();
            }
        }
        let tags = match vet.tag_type() {
            None => None,
            Some(tag_ty) => {
                let (ty, reference) = SQLType::for_cedar_type(tag_ty)?;
                let table = match &ann.tags_table {
                    Some(table) => table.clone(),
                    None => name.with_suffix(TAGS_TABLE_SUFFIX).map_err(|_| {
                        Error::Schema(format!(
                            "the tags table name for {ety} is too long; use @sql_tags_table"
                        ))
                    })?,
                };
                Some((table, ty, reference))
            }
        };
        Ok(Draft {
            name,
            entity_type: ety,
            columns,
            attribute_columns,
            references,
            entity_id_target,
            primary_keys,
            tags,
        })
    }

    fn finish(
        &self,
        mut draft: Draft,
        entity_id_column: &SQLIdentifier,
        entity_type_column: &SQLIdentifier,
        entity_tables: &HashMap<EntityType, SQLIdentifier>,
    ) -> Result<(SQLIdentifier, TableConfiguration)> {
        let ety = draft.entity_type.clone();
        let mut columns = IndexMap::new();
        let entity_id = match &draft.entity_id_target {
            None => {
                let mut column = ColumnConfiguration::new(SQLType::Text);
                // Unique in its own right when another column is the primary key.
                column.unique = !draft.primary_keys.is_empty()
                    && draft.primary_keys.as_slice() != std::slice::from_ref(entity_id_column);
                columns.insert(entity_id_column.clone(), column);
                entity_id_column.clone()
            }
            Some(target) => {
                if !draft.columns.contains_key(target) {
                    // An otherwise undefined name adds a column (the README's rule).
                    let mut added = ColumnConfiguration::new(SQLType::Text);
                    added.unique = true;
                    draft.columns.insert(target.clone(), added);
                }
                let column = &draft.columns[target];
                if column.ty != SQLType::Text || column.nullable {
                    return Err(Error::Schema(format!(
                        "the entity id column {target} of {ety} must be a non-nullable text column"
                    )));
                }
                let is_key =
                    column.unique || draft.primary_keys.as_slice() == std::slice::from_ref(target);
                if !is_key {
                    return Err(Error::Schema(format!(
                        "the entity id column {target} of {ety} must be unique or the primary key"
                    )));
                }
                let mut generated = ColumnConfiguration::new(SQLType::Text);
                generated.generated = Some(target.to_string());
                columns.insert(entity_id_column.clone(), generated);
                target.clone()
            }
        };
        let mut type_column = ColumnConfiguration::new(SQLType::Text);
        type_column.generated = Some(crate::ident::quoted_literal(&ety.to_string())?);
        columns.insert(entity_type_column.clone(), type_column);
        let references = std::mem::take(&mut draft.references);
        for (name, mut column) in std::mem::take(&mut draft.columns) {
            if let Some((_, target)) = references.iter().find(|(c, _)| *c == name) {
                let Some(table) = entity_tables.get(target) else {
                    return Err(Error::Unsupported(
                        "attributes referencing action entity types",
                    ));
                };
                column.references = Some(ForeignKey {
                    table: table.clone(),
                    column: entity_id_column.clone(), // fixed up by `build`
                    entity_type: target.clone(),
                });
            }
            columns.insert(name, column);
        }
        let primary_keys: IndexSet<SQLIdentifier> = if draft.primary_keys.is_empty() {
            IndexSet::from([entity_id.clone()])
        } else {
            for key in &draft.primary_keys {
                if !columns.contains_key(key) {
                    return Err(Error::Schema(format!(
                        "the primary key column {key} of {ety} does not exist"
                    )));
                }
            }
            draft.primary_keys.into_iter().collect()
        };
        let tags = match draft.tags {
            None => None,
            Some((table, ty, reference)) => {
                let mut value = ColumnConfiguration::new(ty);
                if let Some(target) = reference {
                    let Some(table) = entity_tables.get(&target) else {
                        return Err(Error::Unsupported("tags referencing action entity types"));
                    };
                    value.references = Some(ForeignKey {
                        table: table.clone(),
                        column: entity_id_column.clone(), // fixed up by `build`
                        entity_type: target.clone(),
                    });
                }
                Some(TagsTableConfiguration { table, value })
            }
        };
        Ok((
            draft.name,
            TableConfiguration {
                entity_type: ety,
                primary_keys,
                columns,
                tags,
                entity_id_column: entity_id,
                attribute_columns: draft.attribute_columns,
            },
        ))
    }
}

/// `default` unless taken, else `default2`, `default3`, … (the README's rule).
fn unique_name(default: &str, taken: &BTreeSet<&str>) -> Result<SQLIdentifier> {
    if !taken.contains(default) {
        return SQLIdentifier::new(default);
    }
    (2u32..)
        .map(|n| format!("{default}{n}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        .map(SQLIdentifier::new)
        .expect("an unbounded sequence of candidates has a free one")
}
