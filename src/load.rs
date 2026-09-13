//! Turning Cedar entities into rows.
//!
//! [`entities_to_sql`] renders one `INSERT` per entity into its type's table,
//! one per ancestor into the hierarchy table (Cedar entities carry their
//! transitive ancestors, so the table holds the closure), and one per tag
//! into the tags table. Action entities have no table; they are returned
//! separately so that partial evaluation can be given them.
//!
//! Compound values use the canonical JSON encoding of [`canonical_json`]:
//! a set is an array of its (deduplicated) elements, a record is
//! `{"r": {…}}`, an entity reference is `{"e": {"t": type, "i": id}}`, and
//! primitives are JSON primitives. The wrappers keep records, entities and
//! sets apart whatever the record's keys are.

use cedar_policy::{Entities, Schema};
use cedar_policy_core::ast::{
    Entity, EntityType, EntityUID, Literal, PartialValue, Value, ValueKind,
};
use cedar_policy_core::validator::ValidatorSchema;
use cedar_policy_core::validator::types::{EntityKind, OpenTag, Type};
use serde_json::json;

use crate::config::{
    ColumnConfiguration, DatabaseConfiguration, HIERARCHY_COLUMNS, SQLType, TAGS_ENTITY_ID_COLUMN,
    TAGS_TAG_COLUMN, TAGS_VALUE_COLUMN,
};
use crate::dialect::Dialect;
use crate::ident::quoted_literal;
use crate::{Error, Result};

/// The statements loading a set of entities, and the action entities.
#[derive(Debug, Default)]
pub struct Load {
    /// `INSERT` statements, in an order that satisfies the foreign keys only
    /// when the referenced entities exist (Cedar allows dangling references).
    pub statements: Vec<String>,
    /// The action entities, which have no table.
    pub actions: Vec<Entity>,
}

/// Renders `entities` as rows of the tables of `config`.
pub fn entities_to_sql(
    entities: &Entities,
    schema: &Schema,
    config: &DatabaseConfiguration,
    dialect: &dyn Dialect,
) -> Result<Load> {
    let vschema: &ValidatorSchema = schema.as_ref();
    let mut load = Load::default();
    let mut ordered: Vec<&Entity> = entities.as_ref().iter().collect();
    ordered.sort_by_key(|e| e.uid().to_string());
    for entity in ordered {
        let uid = entity.uid();
        let ety = uid.entity_type();
        if ety.is_action() {
            load.actions.push(entity.clone());
            continue;
        }
        let Some((table_name, table)) = config.table_for(ety) else {
            return Err(Error::Load(format!("no table stores entity type {ety}")));
        };
        let Some(vet) = vschema.get_entity_type(ety) else {
            return Err(Error::Load(format!("the schema has no entity type {ety}")));
        };
        let eid = quoted_literal(uid.eid().as_ref())?;
        let mut columns = vec![table.entity_id_column.to_string()];
        let mut values = vec![eid.clone()];
        for (attr, column) in &table.attribute_columns {
            if *column == table.entity_id_column {
                // The id column is also exposed as an attribute; the values must agree.
                match entity.get(attr) {
                    Some(PartialValue::Value(v)) if matches!(v.value_kind(), ValueKind::Lit(Literal::String(s)) if s.as_str() == uid.eid().as_ref()) =>
                        {}
                    _ => {
                        return Err(Error::Load(format!(
                            "attribute {attr} of {uid} must equal the entity id, since it is the entity id column"
                        )));
                    }
                }
                continue;
            }
            let cc = &table.columns[column];
            let rendered = match entity.get(attr) {
                None if cc.nullable => "NULL".to_owned(),
                None => {
                    return Err(Error::Load(format!(
                        "required attribute {attr} of {uid} is missing"
                    )));
                }
                Some(PartialValue::Value(v)) => {
                    if let Some(attr_type) = vet.attr(attr) {
                        conforms(v, &attr_type.attr_type)
                            .map_err(|e| Error::Load(format!("attribute {attr} of {uid}: {e}")))?;
                    }
                    literal(v, cc, dialect)?
                }
                Some(PartialValue::Residual(_)) => {
                    return Err(Error::Unsupported("entities with unknown attribute values"));
                }
            };
            columns.push(column.to_string());
            values.push(rendered);
        }
        for (attr, _) in entity.attrs() {
            if vet.attr(attr).is_none() {
                return Err(Error::Load(format!(
                    "attribute {attr} of {uid} is not declared in the schema"
                )));
            }
        }
        load.statements.push(format!(
            "INSERT INTO {table_name} ({}) VALUES ({})",
            columns.join(", "),
            values.join(", ")
        ));
        let mut ancestors: Vec<&EntityUID> = entity.ancestors().collect();
        ancestors.sort_by_key(|a| a.to_string());
        for ancestor in ancestors {
            load.statements.push(format!(
                "INSERT INTO {} ({}) VALUES ({}, {eid}, {}, {})",
                config.entity_hierarchy_table,
                HIERARCHY_COLUMNS
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", "),
                quoted_literal(&ety.to_string())?,
                quoted_literal(&ancestor.entity_type().to_string())?,
                quoted_literal(ancestor.eid().as_ref())?,
            ));
        }
        let mut tags: Vec<(&str, &PartialValue)> =
            entity.tags().map(|(k, v)| (k.as_str(), v)).collect();
        tags.sort_by_key(|(k, _)| *k);
        for (tag, value) in tags {
            let Some(tags_table) = &table.tags else {
                return Err(Error::Load(format!(
                    "{uid} has tag {tag}, but its type declares no tags"
                )));
            };
            let PartialValue::Value(value) = value else {
                return Err(Error::Unsupported("entities with unknown tag values"));
            };
            if let Some(tag_type) = vet.tag_type() {
                conforms(value, tag_type)
                    .map_err(|e| Error::Load(format!("tag {tag} of {uid}: {e}")))?;
            }
            load.statements.push(format!(
                "INSERT INTO {} (\"{TAGS_ENTITY_ID_COLUMN}\", \"{TAGS_TAG_COLUMN}\", \"{TAGS_VALUE_COLUMN}\") VALUES ({eid}, {}, {})",
                tags_table.table,
                quoted_literal(tag)?,
                literal(value, &tags_table.value, dialect)?,
            ));
        }
    }
    Ok(load)
}

/// Whether `value` conforms to the validator type `ty`: the compiled queries
/// trust the static types of JSON-stored contents (entity references inside
/// sets and records are compared by id), so the loader checks them.
pub fn conforms(value: &Value, ty: &Type) -> std::result::Result<(), String> {
    let mismatch = || format!("the value {value} does not conform to the type {ty}");
    match (ty, value.value_kind()) {
        (Type::Bool(_), ValueKind::Lit(Literal::Bool(_)))
        | (Type::Long, ValueKind::Lit(Literal::Long(_)))
        | (Type::String, ValueKind::Lit(Literal::String(_)))
        | (Type::Entity(EntityKind::AnyEntity), ValueKind::Lit(Literal::EntityUID(_)))
        | (Type::ExtensionType { .. }, ValueKind::ExtensionValue(_)) => Ok(()),
        (Type::Entity(EntityKind::Entity(lub)), ValueKind::Lit(Literal::EntityUID(uid))) => {
            match lub.get_single_entity() {
                Some(ety) if ety == uid.entity_type() => Ok(()),
                Some(_) => Err(mismatch()),
                None => Err("union entity types are not supported".to_owned()),
            }
        }
        (Type::Set { element_type }, ValueKind::Set(set)) => match element_type {
            Some(element) => set.iter().try_for_each(|v| conforms(v, element)),
            None => Ok(()),
        },
        (
            Type::Record {
                attrs,
                open_attributes,
            },
            ValueKind::Record(record),
        ) => {
            for (key, attr) in attrs.iter() {
                match record.get(key) {
                    Some(v) => conforms(v, &attr.attr_type)?,
                    None if attr.is_required => {
                        return Err(format!(
                            "the required attribute {key} is missing in {value}"
                        ));
                    }
                    None => {}
                }
            }
            if *open_attributes == OpenTag::ClosedAttributes
                && let Some(extra) = record.keys().find(|k| attrs.get_attr(k).is_none())
            {
                return Err(format!("the attribute {extra} of {value} is not declared"));
            }
            Ok(())
        }
        _ => Err(mismatch()),
    }
}

/// `value` as a SQL literal for `column`: strings and entity references of
/// the referenced type in text columns, integers, booleans, and the canonical
/// JSON of sets and records.
pub fn literal(
    value: &Value,
    column: &ColumnConfiguration,
    dialect: &dyn Dialect,
) -> Result<String> {
    let mismatch = || {
        Error::Load(format!(
            "the value {value} cannot be stored in a column of type {:?}{}",
            column.ty,
            column
                .references
                .as_ref()
                .map(|fk| format!(" referencing {}", fk.entity_type))
                .unwrap_or_default()
        ))
    };
    match (&column.ty, value.value_kind()) {
        (SQLType::Text, ValueKind::Lit(Literal::String(s))) if column.references.is_none() => {
            quoted_literal(s)
        }
        (SQLType::Text, ValueKind::Lit(Literal::EntityUID(uid))) => match &column.references {
            Some(fk) if fk.entity_type == *uid.entity_type() => quoted_literal(uid.eid().as_ref()),
            _ => Err(mismatch()),
        },
        (SQLType::BigInt, ValueKind::Lit(Literal::Long(n))) => Ok(dialect.bigint_literal(*n)),
        (SQLType::Bool, ValueKind::Lit(Literal::Bool(b))) => {
            Ok(if *b { "TRUE" } else { "FALSE" }.to_owned())
        }
        (SQLType::Jsonb, ValueKind::Set(_) | ValueKind::Record(_)) => {
            let json = canonical_json(value)?;
            Ok(dialect.json_literal(&quoted_literal(&json.to_string())?))
        }
        (SQLType::Set(_) | SQLType::Custom(_), _) => Err(Error::Unsupported(
            "loading values into array or custom columns",
        )),
        (_, ValueKind::ExtensionValue(_)) => Err(Error::Unsupported("extension values")),
        _ => Err(mismatch()),
    }
}

/// The canonical JSON encoding of a Cedar value (see the module docs).
pub fn canonical_json(value: &Value) -> Result<serde_json::Value> {
    Ok(match value.value_kind() {
        ValueKind::Lit(Literal::Bool(b)) => json!(b),
        ValueKind::Lit(Literal::Long(n)) => json!(n),
        ValueKind::Lit(Literal::String(s)) => {
            if s.contains('\0') {
                return Err(Error::Nul(s.to_string()));
            }
            json!(s.as_str())
        }
        ValueKind::Lit(Literal::EntityUID(uid)) => entity_json(uid),
        ValueKind::Set(set) => {
            serde_json::Value::Array(set.iter().map(canonical_json).collect::<Result<Vec<_>>>()?)
        }
        ValueKind::Record(record) => {
            let fields = record
                .iter()
                .map(|(k, v)| {
                    if k.contains('\0') {
                        return Err(Error::Nul(k.to_string()));
                    }
                    Ok((k.to_string(), canonical_json(v)?))
                })
                .collect::<Result<serde_json::Map<String, serde_json::Value>>>()?;
            json!({ "r": fields })
        }
        ValueKind::ExtensionValue(_) => return Err(Error::Unsupported("extension values")),
    })
}

/// The canonical JSON encoding of an entity reference.
pub fn entity_json(uid: &EntityUID) -> serde_json::Value {
    json!({ "e": { "t": uid.entity_type().to_string(), "i": uid.eid().as_ref() } })
}

/// The entity type of a canonical entity reference, for messages and tests.
pub fn entity_type_of_json(value: &serde_json::Value) -> Option<EntityType> {
    value.get("e")?.get("t")?.as_str()?.parse().ok()
}
