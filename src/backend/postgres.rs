//! The Postgres backend.

use postgres::types::Type;
use postgres::{Client, NoTls};

use super::{Backend, Row, SqlValue};
use crate::{Error, Result};

/// A synchronous Postgres connection.
pub struct PgBackend {
    client: Client,
}

impl PgBackend {
    /// Connects to `url` (a `postgres://` URL or a key-value connection string).
    pub fn connect(url: &str) -> Result<Self> {
        Ok(Self {
            client: Client::connect(url, NoTls)?,
        })
    }

    /// The underlying client.
    pub fn client_mut(&mut self) -> &mut Client {
        &mut self.client
    }
}

impl Backend for PgBackend {
    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        Ok(self.client.batch_execute(sql)?)
    }

    fn query(&mut self, sql: &str) -> Result<Vec<Row>> {
        let rows = self.client.query(sql, &[])?;
        rows.iter()
            .map(|row| {
                row.columns()
                    .iter()
                    .enumerate()
                    .map(|(i, column)| decode(row, i, column.type_()))
                    .collect()
            })
            .collect()
    }
}

fn decode(row: &postgres::Row, i: usize, ty: &Type) -> Result<SqlValue> {
    let decode_err = |e: postgres::Error| Error::Decode {
        column: i,
        message: e.to_string(),
    };
    Ok(match *ty {
        Type::BOOL => row
            .try_get::<_, Option<bool>>(i)
            .map_err(decode_err)?
            .map_or(SqlValue::Null, SqlValue::Bool),
        Type::INT8 => row
            .try_get::<_, Option<i64>>(i)
            .map_err(decode_err)?
            .map_or(SqlValue::Null, SqlValue::Long),
        Type::INT4 => row
            .try_get::<_, Option<i32>>(i)
            .map_err(decode_err)?
            .map_or(SqlValue::Null, |v| SqlValue::Long(v.into())),
        Type::TEXT | Type::VARCHAR => row
            .try_get::<_, Option<String>>(i)
            .map_err(decode_err)?
            .map_or(SqlValue::Null, SqlValue::Text),
        Type::JSONB | Type::JSON => row
            .try_get::<_, Option<serde_json::Value>>(i)
            .map_err(decode_err)?
            .map_or(SqlValue::Null, SqlValue::Json),
        ref other => {
            return Err(Error::Decode {
                column: i,
                message: format!("unsupported column type {other}"),
            });
        }
    })
}
