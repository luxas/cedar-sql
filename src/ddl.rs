//! Rendering a [`DatabaseConfiguration`] as DDL.

use crate::Result;
use crate::config::ColumnConfiguration;
use crate::config::{
    DatabaseConfiguration, HIERARCHY_COLUMNS, TAGS_ENTITY_ID_COLUMN, TAGS_TAG_COLUMN,
    TAGS_VALUE_COLUMN, TableConfiguration,
};
use crate::dialect::Dialect;
use crate::ident::{SQLIdentifier, shortened};

/// The statements creating every table of `config`: the entity tables, their
/// tags tables, the hierarchy table, and then the foreign keys as
/// `ALTER TABLE` statements (when `config.emit_foreign_keys`), so that table
/// order and reference cycles do not matter. The foreign keys are deferred
/// to the end of the transaction, so rows may be loaded in any order.
pub fn create_tables(config: &DatabaseConfiguration, dialect: &dyn Dialect) -> Result<Vec<String>> {
    let mut statements = Vec::new();
    let mut foreign_keys = Vec::new();
    for (name, table) in &config.tables {
        statements.push(create_entity_table(name, table, dialect));
        for (column, cc) in &table.columns {
            if let Some(fk) = &cc.references {
                foreign_keys.push(alter_foreign_key(name, column, &fk.table, &fk.column)?);
            }
        }
        if let Some(tags) = &table.tags {
            let value = &tags.value;
            let value_ty = dialect.render_type(&value.ty);
            statements.push(format!(
                "CREATE TABLE {} (\n  \"{TAGS_ENTITY_ID_COLUMN}\" TEXT NOT NULL,\n  \"{TAGS_TAG_COLUMN}\" TEXT NOT NULL,\n  \"{TAGS_VALUE_COLUMN}\" {value_ty} NOT NULL,\n  PRIMARY KEY (\"{TAGS_ENTITY_ID_COLUMN}\", \"{TAGS_TAG_COLUMN}\")\n)",
                tags.table
            ));
            let entity_id = SQLIdentifier::new(TAGS_ENTITY_ID_COLUMN)?;
            foreign_keys.push(alter_foreign_key(
                &tags.table,
                &entity_id,
                name,
                &table.entity_id_column,
            )?);
            if let Some(fk) = &value.references {
                let value_column = SQLIdentifier::new(TAGS_VALUE_COLUMN)?;
                foreign_keys.push(alter_foreign_key(
                    &tags.table,
                    &value_column,
                    &fk.table,
                    &fk.column,
                )?);
            }
        }
    }
    let hierarchy_columns = HIERARCHY_COLUMNS
        .iter()
        .map(|c| format!("  \"{c}\" TEXT NOT NULL,\n"))
        .collect::<String>();
    let hierarchy_key = HIERARCHY_COLUMNS
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    statements.push(format!(
        "CREATE TABLE {} (\n{hierarchy_columns}  PRIMARY KEY ({hierarchy_key})\n)",
        config.entity_hierarchy_table
    ));
    if config.emit_foreign_keys {
        statements.extend(foreign_keys);
    }
    Ok(statements)
}

/// The statements dropping every table of `config`, if they exist.
pub fn drop_tables(config: &DatabaseConfiguration) -> Vec<String> {
    let mut names = vec![config.entity_hierarchy_table.clone()];
    for (name, table) in config.tables.iter().rev() {
        if let Some(tags) = &table.tags {
            names.push(tags.table.clone());
        }
        names.push(name.clone());
    }
    names
        .into_iter()
        .map(|name| format!("DROP TABLE IF EXISTS {name} CASCADE"))
        .collect()
}

fn create_entity_table(
    name: &SQLIdentifier,
    table: &TableConfiguration,
    dialect: &dyn Dialect,
) -> String {
    let mut lines: Vec<String> = table
        .columns
        .iter()
        .map(|(column, cc)| format!("  {column} {}", column_definition(cc, dialect)))
        .collect();
    let keys = table
        .primary_keys
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("  PRIMARY KEY ({keys})"));
    format!("CREATE TABLE {name} (\n{}\n)", lines.join(",\n"))
}

fn column_definition(cc: &ColumnConfiguration, dialect: &dyn Dialect) -> String {
    let mut parts = vec![dialect.render_type(&cc.ty)];
    if !cc.nullable {
        parts.push("NOT NULL".into());
    }
    if let Some(expr) = &cc.generated {
        parts.push(dialect.generated_column(expr));
    }
    if cc.unique {
        parts.push("UNIQUE".into());
    }
    if let Some(custom) = &cc.custom_attrs {
        parts.push(custom.clone());
    }
    parts.join(" ")
}

fn alter_foreign_key(
    table: &SQLIdentifier,
    column: &SQLIdentifier,
    target_table: &SQLIdentifier,
    target_column: &SQLIdentifier,
) -> Result<String> {
    let constraint = shortened(&format!("{}_{}_fkey", table.as_str(), column.as_str()));
    // Deferred, so that rows may reference rows loaded later in the same
    // transaction (`User.friend: User`, or two types referencing each other).
    Ok(format!(
        "ALTER TABLE {table} ADD CONSTRAINT {constraint} FOREIGN KEY ({column}) REFERENCES {target_table} ({target_column}) DEFERRABLE INITIALLY DEFERRED"
    ))
}
