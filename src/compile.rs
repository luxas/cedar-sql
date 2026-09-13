//! Compiling TPE residuals into one SQL query.
//!
//! Every residual node becomes a SQL expression with three-valued semantics:
//! `NULL` is a Cedar evaluation error, so `&&`, `||` and `if` are `CASE`
//! forms that keep Cedar's short-circuiting (`false && error` is `false`,
//! `error && false` is an error), arithmetic is guarded so that no bigint
//! overflow reaches Postgres (which would abort the statement), and every
//! `EXISTS` is guarded by a `NULL` check of its inputs, since `EXISTS` never
//! yields `NULL` by itself. Sets and records are the canonical JSON of
//! [`crate::load::canonical_json`], compared with the
//! [`crate::dialect::CEDAR_EQ`] helper (arrays as sets).
//!
//! Entity data is fetched by CTEs: one per *root* — an unknown request
//! variable, or an entity literal that is dereferenced — with one `LEFT JOIN`
//! per entity-typed attribute path under that root (`principal.a.b`), so that
//! each root row yields exactly one row with a column per referenced attribute
//! path. A referenced attribute that is absent, or whose entity row is
//! missing, reads as `NULL`, which is exactly Cedar's error for `getAttr`;
//! `hasAttr` is `IS NOT NULL` guarded by the parent value, so that a missing
//! entity gives `false` and an errored parent gives an error. An entity value
//! that is not such a chain (the result of an `if`, a tag, a record field) is
//! dereferenced by a scalar subquery on its type's table.
//!
//! An operand used more than once (the guard of a `CASE`, an arithmetic
//! result) is bound to a `LATERAL` node, so every sub-expression appears once
//! in the query text and nested `&&` chains do not grow exponentially.
//!
//! [`Compiler::render`] assembles the query: the CTEs, one boolean column per
//! policy, and the `FROM (SELECT 1) CROSS JOIN <unknown roots> LEFT JOIN
//! <literal roots> ON TRUE CROSS JOIN LATERAL <nodes>` skeleton, which yields
//! exactly one row for a concrete request and one row per candidate entity
//! otherwise.

use std::fmt::Write as _;

use cedar_policy_core::ast::{
    BinaryOp, EntityType, EntityUID, Literal, PatternElem, UnaryOp, Value, ValueKind, Var,
};
use cedar_policy_core::tpe::request::PartialRequest;
use cedar_policy_core::tpe::residual::{Residual, ResidualKind};
use cedar_policy_core::validator::ValidatorSchema;
use cedar_policy_core::validator::types::{EntityKind, Type};
use indexmap::{IndexMap, IndexSet};
use smol_str::SmolStr;

use crate::config::{
    DatabaseConfiguration, HIERARCHY_COLUMNS, SQLType, TAGS_ENTITY_ID_COLUMN, TAGS_TAG_COLUMN,
    TAGS_VALUE_COLUMN,
};
use crate::dialect::{CEDAR_EQ, Dialect};
use crate::ident::{SQLIdentifier, quoted_literal, shortened};
use crate::load::canonical_json;
use crate::{Error, Result};

/// The SQL representation of a compiled value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repr {
    /// A `BOOLEAN`.
    Bool,
    /// A `BIGINT`.
    Long,
    /// A `TEXT`.
    Text,
    /// A `TEXT` entity id of the given (statically known) entity type.
    Entity(EntityType),
    /// A `JSONB` in the canonical encoding (a set or a record).
    Json,
}

impl Repr {
    /// The representation of values of a validator type.
    pub fn of(ty: &Type) -> Result<Repr> {
        Ok(match ty {
            Type::Bool(_) => Repr::Bool,
            Type::Long => Repr::Long,
            Type::String => Repr::Text,
            Type::Entity(EntityKind::Entity(lub)) => match lub.get_single_entity() {
                Some(ety) => Repr::Entity(ety.clone()),
                None => return Err(Error::Unsupported("values of a union entity type")),
            },
            Type::Entity(EntityKind::AnyEntity) => {
                return Err(Error::Unsupported("values of an unspecified entity type"));
            }
            Type::Set { .. } | Type::Record { .. } => Repr::Json,
            Type::ExtensionType { .. } => return Err(Error::Unsupported("extension types")),
            Type::Never => return Err(Error::Unsupported("values of the never type")),
        })
    }

    fn sql_type(&self) -> &'static str {
        match self {
            Repr::Bool => "boolean",
            Repr::Long => "bigint",
            Repr::Text | Repr::Entity(_) => "text",
            Repr::Json => "jsonb",
        }
    }
}

/// A root: the source of an entity value that rows are fetched for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RootKey {
    /// The unknown `principal`.
    Principal,
    /// The unknown `resource`.
    Resource,
    /// An entity literal that is dereferenced.
    Literal(EntityUID),
}

/// An entity value that is a row-anchored attribute chain: `root.a.b`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    root: RootKey,
    path: Vec<SmolStr>,
}

/// A compiled residual.
#[derive(Clone, Debug)]
pub struct Compiled {
    /// The SQL expression.
    pub sql: String,
    /// Its representation.
    pub repr: Repr,
    /// When the value is an entity fetched by a root CTE, where it comes from.
    pub anchor: Option<Anchor>,
}

/// What one root's CTE must fetch.
#[derive(Debug)]
struct Root {
    ety: EntityType,
    /// Entity-typed attribute paths that are joined, with the joined type.
    joins: IndexMap<Vec<SmolStr>, EntityType>,
    /// Attribute paths whose values the CTE exports.
    values: IndexSet<Vec<SmolStr>>,
}

/// The per-anchor ancestors CTE, when the hierarchy table is not closed.
#[derive(Debug)]
struct AncestorsCte {
    anchor: Anchor,
    ety: EntityType,
}

/// Compiles residuals against one request, collecting the CTEs they need.
pub struct Compiler<'a> {
    config: &'a DatabaseConfiguration,
    schema: &'a ValidatorSchema,
    dialect: &'a dyn Dialect,
    request: &'a PartialRequest,
    roots: IndexMap<RootKey, Root>,
    ancestors: Vec<AncestorsCte>,
    /// Shared sub-expressions, each a `LATERAL` node `nK` with one column `v`.
    nodes: Vec<String>,
}

impl<'a> Compiler<'a> {
    /// A compiler for `request`.
    pub fn new(
        config: &'a DatabaseConfiguration,
        schema: &'a ValidatorSchema,
        dialect: &'a dyn Dialect,
        request: &'a PartialRequest,
    ) -> Self {
        Self {
            config,
            schema,
            dialect,
            request,
            roots: IndexMap::new(),
            ancestors: Vec::new(),
            nodes: Vec::new(),
        }
    }

    /// Compiles a residual to a boolean SQL expression.
    pub fn condition(&mut self, r: &Residual) -> Result<String> {
        let compiled = self.expr(r)?;
        match compiled.repr {
            Repr::Bool => Ok(compiled.sql),
            _ => Err(Error::Unsupported("a policy condition that is not boolean")),
        }
    }

    /// Compiles a residual.
    pub fn expr(&mut self, r: &Residual) -> Result<Compiled> {
        let repr = Repr::of(r.ty())?;
        let kind = match r {
            Residual::Concrete { value, .. } => return self.value(value, repr),
            Residual::Error(_) => {
                return Ok(plain(format!("NULL::{}", repr.sql_type()), repr));
            }
            Residual::Partial { kind, .. } => kind,
        };
        let sql = match kind {
            ResidualKind::Var(Var::Principal) => return self.var(RootKey::Principal, repr),
            ResidualKind::Var(Var::Resource) => return self.var(RootKey::Resource, repr),
            ResidualKind::Var(Var::Action | Var::Context) => {
                return Err(Error::Unsupported("an unknown action or context"));
            }
            ResidualKind::And { left, right } => {
                let a = self.shared_bool(left)?;
                let b = self.bool(right)?;
                format!("(CASE WHEN {a} IS NULL THEN NULL WHEN {a} THEN {b} ELSE FALSE END)")
            }
            ResidualKind::Or { left, right } => {
                let a = self.shared_bool(left)?;
                let b = self.bool(right)?;
                format!("(CASE WHEN {a} IS NULL THEN NULL WHEN {a} THEN TRUE ELSE {b} END)")
            }
            ResidualKind::If {
                test_expr,
                then_expr,
                else_expr,
            } => {
                let c = self.shared_bool(test_expr)?;
                let t = self.expr(then_expr)?;
                let e = self.expr(else_expr)?;
                if t.repr != repr || e.repr != repr {
                    return Err(Error::Unsupported(
                        "an if whose branches differ in representation",
                    ));
                }
                format!(
                    "(CASE WHEN {c} IS NULL THEN NULL WHEN {c} THEN {} ELSE {} END)",
                    t.sql, e.sql
                )
            }
            ResidualKind::UnaryApp { op, arg } => {
                let a = self.expr(arg)?;
                match op {
                    UnaryOp::Not => format!("(NOT {})", a.sql),
                    UnaryOp::Neg => {
                        let a = self.share(a);
                        format!(
                            "(CASE WHEN {a} = {min} THEN NULL ELSE -{a} END)",
                            a = a.sql,
                            min = self.dialect.bigint_literal(i64::MIN)
                        )
                    }
                    UnaryOp::IsEmpty => {
                        if a.repr != Repr::Json {
                            return Err(Error::Unsupported("isEmpty on a non-set"));
                        }
                        format!("(jsonb_array_length({}) = 0)", a.sql)
                    }
                }
            }
            ResidualKind::BinaryApp { op, arg1, arg2 } => {
                return self.binary(*op, arg1, arg2, repr);
            }
            ResidualKind::GetAttr { expr, attr } => return self.get_attr(expr, attr, repr),
            ResidualKind::HasAttr { expr, attr } => self.has_attr(expr, attr)?,
            ResidualKind::Like { expr, pattern } => {
                let a = self.expr(expr)?;
                let mut like = String::new();
                for elem in pattern.iter() {
                    match elem {
                        PatternElem::Wildcard => like.push('%'),
                        PatternElem::Char(c @ ('%' | '_' | '\\')) => {
                            like.push('\\');
                            like.push(*c);
                        }
                        PatternElem::Char(c) => like.push(*c),
                    }
                }
                format!("({} LIKE {} ESCAPE '\\')", a.sql, quoted_literal(&like)?)
            }
            ResidualKind::Is { expr, entity_type } => {
                let a = self.expr(expr)?;
                let Repr::Entity(ety) = &a.repr else {
                    return Err(Error::Unsupported("is on a non-entity value"));
                };
                let outcome = if ety == entity_type { "TRUE" } else { "FALSE" };
                format!("(CASE WHEN {} IS NULL THEN NULL ELSE {outcome} END)", a.sql)
            }
            ResidualKind::ExtensionFunctionApp { .. } => {
                return Err(Error::Unsupported("extension functions"));
            }
            ResidualKind::Set(items) => {
                let mut elements = Vec::new();
                let mut guards = Vec::new();
                for item in items.iter() {
                    let c = self.expr(item)?;
                    let c = self.share(c);
                    guards.push(format!("{} IS NULL", c.sql));
                    elements.push(self.to_json(&c));
                }
                if elements.is_empty() {
                    "'[]'::jsonb".to_owned()
                } else {
                    format!(
                        "(CASE WHEN {} THEN NULL ELSE jsonb_build_array({}) END)",
                        guards.join(" OR "),
                        elements.join(", ")
                    )
                }
            }
            ResidualKind::Record(fields) => {
                let mut pairs = Vec::new();
                let mut guards = Vec::new();
                for (key, field) in fields.iter() {
                    let c = self.expr(field)?;
                    let c = self.share(c);
                    guards.push(format!("{} IS NULL", c.sql));
                    pairs.push(format!("{}, {}", quoted_literal(key)?, self.to_json(&c)));
                }
                let record = if pairs.is_empty() {
                    "'{}'::jsonb".to_owned()
                } else {
                    format!("jsonb_build_object({})", pairs.join(", "))
                };
                if guards.is_empty() {
                    format!("jsonb_build_object('r', {record})")
                } else {
                    format!(
                        "(CASE WHEN {} THEN NULL ELSE jsonb_build_object('r', {record}) END)",
                        guards.join(" OR ")
                    )
                }
            }
        };
        Ok(plain(sql, repr))
    }

    fn bool(&mut self, r: &Residual) -> Result<String> {
        let c = self.expr(r)?;
        match c.repr {
            Repr::Bool => Ok(c.sql),
            _ => Err(Error::Unsupported(
                "a non-boolean operand of a boolean operator",
            )),
        }
    }

    /// A boolean operand that the caller uses more than once.
    fn shared_bool(&mut self, r: &Residual) -> Result<String> {
        let c = self.expr(r)?;
        match c.repr {
            Repr::Bool => Ok(self.share(c).sql),
            _ => Err(Error::Unsupported(
                "a non-boolean operand of a boolean operator",
            )),
        }
    }

    /// Binds `c` to a `LATERAL` node unless it is already a reference or a
    /// literal, so that its expression appears once in the query.
    fn share(&mut self, c: Compiled) -> Compiled {
        let sql = c.sql.trim();
        let is_reference = sql.starts_with('"') && !sql.contains(' ');
        let is_literal =
            sql.starts_with('\'') || sql == "TRUE" || sql == "FALSE" || sql.starts_with("NULL::");
        if is_reference || is_literal {
            return c;
        }
        let index = self.nodes.len();
        self.nodes.push(c.sql);
        Compiled {
            sql: format!("\"n{index}\".\"v\""),
            repr: c.repr,
            anchor: c.anchor,
        }
    }

    /// `c` in the canonical JSON encoding (`NULL` stays `NULL`).
    fn to_json(&self, c: &Compiled) -> String {
        match &c.repr {
            // Cast explicitly: a bare literal has an unknown type.
            Repr::Bool => format!("to_jsonb(({})::boolean)", c.sql),
            Repr::Long => format!("to_jsonb(({})::bigint)", c.sql),
            Repr::Text => format!("to_jsonb(({})::text)", c.sql),
            Repr::Entity(ety) => format!(
                "(CASE WHEN {v} IS NULL THEN NULL ELSE jsonb_build_object('e', jsonb_build_object('t', {t}, 'i', {v})) END)",
                v = c.sql,
                t = quoted_literal(&ety.to_string()).expect("type names have no NUL")
            ),
            Repr::Json => c.sql.clone(),
        }
    }

    fn value(&mut self, value: &Value, repr: Repr) -> Result<Compiled> {
        let sql = match (value.value_kind(), &repr) {
            (ValueKind::Lit(Literal::Bool(b)), Repr::Bool) => {
                if *b { "TRUE" } else { "FALSE" }.to_owned()
            }
            (ValueKind::Lit(Literal::Long(n)), Repr::Long) => self.dialect.bigint_literal(*n),
            (ValueKind::Lit(Literal::String(s)), Repr::Text) => quoted_literal(s)?,
            (ValueKind::Lit(Literal::EntityUID(uid)), Repr::Entity(ety)) => {
                if uid.entity_type() != ety {
                    return Err(Error::Unsupported(
                        "an entity literal typed as another entity type",
                    ));
                }
                return Ok(Compiled {
                    sql: quoted_literal(uid.eid().as_ref())?,
                    repr,
                    anchor: Some(Anchor {
                        root: RootKey::Literal(uid.as_ref().clone()),
                        path: Vec::new(),
                    }),
                });
            }
            (ValueKind::Set(_) | ValueKind::Record(_), Repr::Json) => {
                let json = canonical_json(value)?;
                self.dialect
                    .json_literal(&quoted_literal(&json.to_string())?)
            }
            (ValueKind::ExtensionValue(_), _) => {
                return Err(Error::Unsupported("extension values"));
            }
            _ => {
                return Err(Error::Unsupported(
                    "a value whose type disagrees with its representation",
                ));
            }
        };
        Ok(plain(sql, repr))
    }

    fn var(&mut self, root: RootKey, repr: Repr) -> Result<Compiled> {
        let Repr::Entity(ety) = &repr else {
            return Err(Error::Unsupported(
                "a request variable that is not an entity",
            ));
        };
        let expected = match root {
            RootKey::Principal => self.request.principal_type(),
            RootKey::Resource => self.request.resource_type(),
            RootKey::Literal(_) => unreachable!("only variables are passed"),
        };
        if expected != ety {
            return Err(Error::Unsupported(
                "a request variable typed as another entity type",
            ));
        }
        self.ensure_root(&root, ety);
        Ok(Compiled {
            sql: format!("{}.{}", root_name(&root), ID_COLUMN),
            repr,
            anchor: Some(Anchor {
                root,
                path: Vec::new(),
            }),
        })
    }

    fn ensure_root(&mut self, root: &RootKey, ety: &EntityType) {
        self.roots.entry(root.clone()).or_insert_with(|| Root {
            ety: ety.clone(),
            joins: IndexMap::new(),
            values: IndexSet::new(),
        });
    }

    fn root_type(&self, anchor: &Anchor) -> EntityType {
        match &anchor.root {
            RootKey::Principal => self.request.principal_type().clone(),
            RootKey::Resource => self.request.resource_type().clone(),
            RootKey::Literal(uid) => uid.entity_type().clone(),
        }
    }

    /// The column of `anchor`'s attribute `attr`, registering the joins and
    /// the exported value.
    fn anchored_attr(
        &mut self,
        anchor: &Anchor,
        ety: &EntityType,
        attr: &SmolStr,
    ) -> Result<String> {
        if self.config.column_for(ety, attr).is_none() {
            return Err(Error::Unsupported("an attribute without a column"));
        }
        let root_type = self.root_type(anchor);
        self.ensure_root(&anchor.root, &root_type);
        let root = self.roots.get_mut(&anchor.root).expect("just ensured");
        if !anchor.path.is_empty() {
            root.joins.entry(anchor.path.clone()).or_insert(ety.clone());
        }
        let mut path = anchor.path.clone();
        path.push(attr.clone());
        root.values.insert(path.clone());
        Ok(format!(
            "{}.{}",
            root_name(&anchor.root),
            value_column(&path)
        ))
    }

    /// The table and entity id column of `ety`.
    fn table_of(&self, ety: &EntityType) -> Result<(&SQLIdentifier, &SQLIdentifier)> {
        match self.config.table_for(ety) {
            Some((name, table)) => Ok((name, &table.entity_id_column)),
            None => Err(Error::Unsupported("an entity type without a table")),
        }
    }

    fn get_attr(&mut self, expr: &Residual, attr: &SmolStr, repr: Repr) -> Result<Compiled> {
        let inner = self.expr(expr)?;
        match &inner.repr {
            Repr::Entity(ety) => {
                let ety = ety.clone();
                if let Some(anchor) = &inner.anchor {
                    let sql = self.anchored_attr(anchor, &ety, attr)?;
                    let anchor = match &repr {
                        Repr::Entity(_) => {
                            let mut path = anchor.path.clone();
                            path.push(attr.clone());
                            Some(Anchor {
                                root: anchor.root.clone(),
                                path,
                            })
                        }
                        _ => None,
                    };
                    return Ok(Compiled { sql, repr, anchor });
                }
                // A computed entity: one row at most, `NULL` when missing.
                let Some(column) = self.config.column_for(&ety, attr).cloned() else {
                    return Err(Error::Unsupported("an attribute without a column"));
                };
                let (table, id_column) = self.table_of(&ety)?;
                let sql = format!(
                    "(SELECT t.{column} FROM {table} AS t WHERE t.{id_column} = {})",
                    inner.sql
                );
                Ok(plain(sql, repr))
            }
            Repr::Json => {
                let field = format!("({} -> 'r' -> {})", inner.sql, quoted_literal(attr)?);
                Ok(plain(json_to_repr(&field, &repr), repr))
            }
            _ => Err(Error::Unsupported(
                "attribute access on a non-entity, non-record value",
            )),
        }
    }

    fn has_attr(&mut self, expr: &Residual, attr: &SmolStr) -> Result<String> {
        let inner = self.expr(expr)?;
        match &inner.repr {
            Repr::Entity(ety) => {
                let ety = ety.clone();
                let inner = self.share(inner);
                if self
                    .schema
                    .get_entity_type(&ety)
                    .is_none_or(|vet| vet.attr(attr).is_none())
                {
                    // The attribute is not declared, so no entity of the type has it.
                    return Ok(format!(
                        "(CASE WHEN {} IS NULL THEN NULL ELSE FALSE END)",
                        inner.sql
                    ));
                }
                if let Some(anchor) = &inner.anchor {
                    let column = self.anchored_attr(anchor, &ety, attr)?;
                    return Ok(format!(
                        "(CASE WHEN {} IS NULL THEN NULL ELSE {column} IS NOT NULL END)",
                        inner.sql
                    ));
                }
                let Some(column) = self.config.column_for(&ety, attr).cloned() else {
                    return Err(Error::Unsupported("an attribute without a column"));
                };
                let (table, id_column) = self.table_of(&ety)?;
                // No row (a missing entity) is `false`, like an absent attribute.
                Ok(format!(
                    "(CASE WHEN {v} IS NULL THEN NULL ELSE COALESCE((SELECT t.{column} IS NOT NULL FROM {table} AS t WHERE t.{id_column} = {v}), FALSE) END)",
                    v = inner.sql
                ))
            }
            Repr::Json => Ok(format!(
                "(({} -> 'r') ? {})",
                inner.sql,
                quoted_literal(attr)?
            )),
            _ => Err(Error::Unsupported("has on a non-entity, non-record value")),
        }
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        arg1: &Residual,
        arg2: &Residual,
        repr: Repr,
    ) -> Result<Compiled> {
        let a = self.expr(arg1)?;
        let b = self.expr(arg2)?;
        let sql = match op {
            BinaryOp::Eq => match (&a.repr, &b.repr) {
                (Repr::Bool, Repr::Bool) | (Repr::Long, Repr::Long) | (Repr::Text, Repr::Text) => {
                    format!("({} = {})", a.sql, b.sql)
                }
                (Repr::Entity(t1), Repr::Entity(t2)) if t1 == t2 => {
                    format!("({} = {})", a.sql, b.sql)
                }
                (Repr::Json, Repr::Json) => format!("{CEDAR_EQ}({}, {})", a.sql, b.sql),
                // Values of different types are never equal, but an error stays one.
                _ => format!(
                    "(CASE WHEN {} IS NULL OR {} IS NULL THEN NULL ELSE FALSE END)",
                    a.sql, b.sql
                ),
            },
            BinaryOp::Less | BinaryOp::LessEq => {
                if a.repr != Repr::Long || b.repr != Repr::Long {
                    return Err(Error::Unsupported("comparison of non-integers"));
                }
                let cmp = if op == BinaryOp::Less { "<" } else { "<=" };
                format!("({} {cmp} {})", a.sql, b.sql)
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                if a.repr != Repr::Long || b.repr != Repr::Long {
                    return Err(Error::Unsupported("arithmetic on non-integers"));
                }
                let sym = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Sub => "-",
                    _ => "*",
                };
                // Computed in `numeric`, which cannot overflow, then range-checked.
                let r = self.share(plain(
                    format!("({}::numeric {sym} {}::numeric)", a.sql, b.sql),
                    Repr::Long,
                ));
                format!(
                    "(CASE WHEN {r} BETWEEN {min} AND {max} THEN {r}::bigint ELSE NULL END)",
                    r = r.sql,
                    min = i64::MIN,
                    max = i64::MAX
                )
            }
            BinaryOp::In => return self.in_op(a, b, repr),
            BinaryOp::HasTag | BinaryOp::GetTag => {
                let Repr::Entity(ety) = &a.repr else {
                    return Err(Error::Unsupported("tags of a non-entity value"));
                };
                if b.repr != Repr::Text {
                    return Err(Error::Unsupported("a non-string tag"));
                }
                let Some((_, table)) = self.config.table_for(ety) else {
                    return Err(Error::Unsupported("tags of an entity type without a table"));
                };
                let Some(tags) = table.tags.clone() else {
                    return Err(Error::Unsupported("tags of an entity type without tags"));
                };
                let (a, b) = (self.share(a), self.share(b));
                let lookup = format!(
                    "FROM {} AS t WHERE t.\"{TAGS_ENTITY_ID_COLUMN}\" = {} AND t.\"{TAGS_TAG_COLUMN}\" = {}",
                    tags.table, a.sql, b.sql
                );
                if op == BinaryOp::HasTag {
                    format!(
                        "(CASE WHEN {} IS NULL OR {} IS NULL THEN NULL ELSE EXISTS (SELECT 1 {lookup}) END)",
                        a.sql, b.sql
                    )
                } else {
                    let agrees = matches!(
                        (&tags.value.ty, &repr),
                        (SQLType::Text, Repr::Text | Repr::Entity(_))
                            | (SQLType::Bool, Repr::Bool)
                            | (SQLType::BigInt, Repr::Long)
                            | (SQLType::Jsonb, Repr::Json)
                    );
                    if !agrees {
                        return Err(Error::Unsupported(
                            "a tag value whose column type disagrees with its type",
                        ));
                    }
                    // No row (missing entity or tag), or a NULL input: NULL, an error.
                    format!("(SELECT t.\"{TAGS_VALUE_COLUMN}\" {lookup})")
                }
            }
            BinaryOp::Contains => {
                if a.repr != Repr::Json {
                    return Err(Error::Unsupported("contains on a non-set"));
                }
                let (a, b) = (self.share(a), self.share(b));
                let element = self.to_json(&b);
                format!(
                    "(CASE WHEN {a} IS NULL OR {b} IS NULL THEN NULL ELSE EXISTS (SELECT 1 FROM jsonb_array_elements({a}) AS el WHERE {CEDAR_EQ}(el.value, {element})) END)",
                    a = a.sql,
                    b = b.sql
                )
            }
            BinaryOp::ContainsAll | BinaryOp::ContainsAny => {
                if a.repr != Repr::Json || b.repr != Repr::Json {
                    return Err(Error::Unsupported("containsAll/containsAny on non-sets"));
                }
                let (a, b) = (self.share(a), self.share(b));
                let check = if op == BinaryOp::ContainsAll {
                    // Every element of `b` has an equal element in `a`.
                    format!(
                        "NOT EXISTS (SELECT 1 FROM jsonb_array_elements({b}) AS y WHERE NOT EXISTS (SELECT 1 FROM jsonb_array_elements({a}) AS x WHERE {CEDAR_EQ}(x.value, y.value)))",
                        a = a.sql,
                        b = b.sql
                    )
                } else {
                    format!(
                        "EXISTS (SELECT 1 FROM jsonb_array_elements({a}) AS x JOIN jsonb_array_elements({b}) AS y ON {CEDAR_EQ}(x.value, y.value))",
                        a = a.sql,
                        b = b.sql
                    )
                };
                format!(
                    "(CASE WHEN {} IS NULL OR {} IS NULL THEN NULL ELSE {check} END)",
                    a.sql, b.sql
                )
            }
        };
        Ok(plain(sql, repr))
    }

    fn in_op(&mut self, a: Compiled, b: Compiled, repr: Repr) -> Result<Compiled> {
        let Repr::Entity(t1) = &a.repr else {
            return Err(Error::Unsupported("in on a non-entity value"));
        };
        let t1 = t1.clone();
        let a = self.share(a);
        let b = self.share(b);
        let sql = match &b.repr {
            Repr::Entity(t2) => {
                let t2 = t2.clone();
                let check = self.ancestor_check(&a, &t1, &b.sql, &t2)?;
                format!(
                    "(CASE WHEN {} IS NULL OR {} IS NULL THEN NULL ELSE {check} END)",
                    a.sql, b.sql
                )
            }
            Repr::Json => {
                // A set of entities, literal or computed: `a` is in the set or
                // has an ancestor in it.
                let element_type = "(el.value -> 'e' ->> 't')";
                let element_id = "(el.value -> 'e' ->> 'i')";
                let check = self.ancestor_check_dynamic(&a, &t1, element_type, element_id)?;
                format!(
                    "(CASE WHEN {} IS NULL OR {} IS NULL THEN NULL ELSE EXISTS (SELECT 1 FROM jsonb_array_elements({}) AS el WHERE {check}) END)",
                    a.sql, b.sql, b.sql
                )
            }
            _ => {
                return Err(Error::Unsupported(
                    "in with a non-entity, non-set right side",
                ));
            }
        };
        Ok(plain(sql, repr))
    }

    /// `a in b` for non-NULL `a` (of type `t1`) and `b` (of type `t2`).
    fn ancestor_check(
        &mut self,
        a: &Compiled,
        t1: &EntityType,
        b: &str,
        t2: &EntityType,
    ) -> Result<String> {
        let same = if t1 == t2 {
            format!("{} = {b}", a.sql)
        } else {
            "FALSE".to_owned()
        };
        let ancestor = self.ancestor_exists(a, t1, &quoted_literal(&t2.to_string())?, b)?;
        Ok(format!("({same} OR {ancestor})"))
    }

    /// `a in <entity>` for an entity given by SQL expressions for its type
    /// and id (an element of a set).
    fn ancestor_check_dynamic(
        &mut self,
        a: &Compiled,
        t1: &EntityType,
        b_type: &str,
        b_id: &str,
    ) -> Result<String> {
        let same = format!(
            "({b_type} = {} AND {b_id} = {})",
            quoted_literal(&t1.to_string())?,
            a.sql
        );
        let ancestor = self.ancestor_exists(a, t1, b_type, b_id)?;
        Ok(format!("({same} OR {ancestor})"))
    }

    /// Whether the entity `b_type`/`b_id` (SQL expressions) is an ancestor of
    /// `a` (of type `t1`).
    fn ancestor_exists(
        &mut self,
        a: &Compiled,
        t1: &EntityType,
        b_type: &str,
        b_id: &str,
    ) -> Result<String> {
        let [dt, di, at, ai] = HIERARCHY_COLUMNS;
        if self.config.hierarchy_closed {
            return Ok(format!(
                "EXISTS (SELECT 1 FROM {} AS h WHERE h.\"{dt}\" = {} AND h.\"{di}\" = {} AND h.\"{at}\" = {b_type} AND h.\"{ai}\" = {b_id})",
                self.config.entity_hierarchy_table,
                quoted_literal(&t1.to_string())?,
                a.sql,
            ));
        }
        let Some(anchor) = &a.anchor else {
            return Err(Error::Unsupported(
                "in on a computed entity value over a hierarchy table without its closure",
            ));
        };
        let cte = self.ensure_ancestors(anchor, t1);
        Ok(format!(
            "EXISTS (SELECT 1 FROM {cte} AS anc WHERE anc.{ID_COLUMN} = {}.{ID_COLUMN} AND anc.\"{at}\" = {b_type} AND anc.\"{ai}\" = {b_id})",
            root_name(&anchor.root),
        ))
    }

    fn ensure_ancestors(&mut self, anchor: &Anchor, ety: &EntityType) -> SQLIdentifier {
        let root_type = self.root_type(anchor);
        self.ensure_root(&anchor.root, &root_type);
        if !anchor.path.is_empty() {
            // The chain's value is exported by the parent's alias, whose join
            // `anchored_attr` registered when the chain was compiled.
            let root = self.roots.get_mut(&anchor.root).expect("just ensured");
            root.values.insert(anchor.path.clone());
        }
        if !self.ancestors.iter().any(|c| c.anchor == *anchor) {
            self.ancestors.push(AncestorsCte {
                anchor: anchor.clone(),
                ety: ety.clone(),
            });
        }
        ancestors_name(anchor)
    }

    /// Renders the query: `columns` are `(alias, expression)` pairs selected
    /// after the root ids of the unknown variables.
    pub fn render(&self, columns: &[(String, String)]) -> Result<String> {
        let mut ctes = Vec::new();
        for (key, root) in &self.roots {
            ctes.push(self.render_root(key, root)?);
        }
        for cte in &self.ancestors {
            ctes.push(self.render_ancestors(cte)?);
        }
        let mut sql = String::new();
        if !ctes.is_empty() {
            let recursive = if self.ancestors.is_empty() {
                ""
            } else {
                "RECURSIVE "
            };
            let _ = writeln!(sql, "WITH {recursive}{}", ctes.join(",\n"));
        }
        let mut selected = Vec::new();
        for key in [RootKey::Principal, RootKey::Resource] {
            if self.roots.contains_key(&key) {
                selected.push(format!(
                    "{}.{ID_COLUMN} AS {}",
                    root_name(&key),
                    root_name(&key)
                ));
            }
        }
        for (alias, expr) in columns {
            selected.push(format!("{expr} AS {}", SQLIdentifier::new(alias.as_str())?));
        }
        let _ = write!(
            sql,
            "SELECT {}\nFROM (SELECT 1 AS base) AS base",
            selected.join(",\n  ")
        );
        for key in self.roots.keys() {
            match key {
                RootKey::Principal | RootKey::Resource => {
                    let _ = write!(sql, "\nCROSS JOIN {}", root_name(key));
                }
                RootKey::Literal(_) => {
                    let _ = write!(sql, "\nLEFT JOIN {} ON TRUE", root_name(key));
                }
            }
        }
        for (index, node) in self.nodes.iter().enumerate() {
            let _ = write!(
                sql,
                "\nCROSS JOIN LATERAL (SELECT {node} AS \"v\") AS \"n{index}\""
            );
        }
        Ok(sql)
    }

    fn render_root(&self, key: &RootKey, root: &Root) -> Result<String> {
        let name = root_name(key);
        let Some((table_name, table)) = self.config.table_for(&root.ety) else {
            return Err(Error::Unsupported("an entity type without a table"));
        };
        let mut select = vec![format!("{name}.{} AS {ID_COLUMN}", table.entity_id_column)];
        let mut joins = String::new();
        let mut ordered: Vec<(&Vec<SmolStr>, &EntityType)> = root.joins.iter().collect();
        ordered.sort_by_key(|(path, _)| path.len());
        for (path, ety) in ordered {
            let (attr, prefix) = path.split_last().expect("join paths are non-empty");
            let parent_alias = alias_name(key, prefix);
            let parent_type = if prefix.is_empty() {
                &root.ety
            } else {
                &root.joins[prefix]
            };
            let Some(column) = self.config.column_for(parent_type, attr) else {
                return Err(Error::Unsupported("an attribute without a column"));
            };
            let Some((joined_table, joined)) = self.config.table_for(ety) else {
                return Err(Error::Unsupported("an entity type without a table"));
            };
            let alias = alias_name(key, path);
            let _ = write!(
                joins,
                "\n  LEFT JOIN {joined_table} AS {alias} ON {parent_alias}.{column} = {alias}.{}",
                joined.entity_id_column
            );
        }
        for path in &root.values {
            let (attr, prefix) = path.split_last().expect("value paths are non-empty");
            let owner_type = if prefix.is_empty() {
                &root.ety
            } else {
                &root.joins[prefix]
            };
            let Some(column) = self.config.column_for(owner_type, attr) else {
                return Err(Error::Unsupported("an attribute without a column"));
            };
            select.push(format!(
                "{}.{column} AS {}",
                alias_name(key, prefix),
                value_column(path)
            ));
        }
        let filter = match key {
            RootKey::Literal(uid) => format!(
                "\n  WHERE {name}.{} = {}",
                table.entity_id_column,
                quoted_literal(uid.eid().as_ref())?
            ),
            _ => String::new(),
        };
        Ok(format!(
            "{name} AS (\n  SELECT {}\n  FROM {table_name} AS {name}{joins}{filter}\n)",
            select.join(",\n    ")
        ))
    }

    fn render_ancestors(&self, cte: &AncestorsCte) -> Result<String> {
        let name = ancestors_name(&cte.anchor);
        let root = root_name(&cte.anchor.root);
        let value = if cte.anchor.path.is_empty() {
            format!("{root}.{ID_COLUMN}")
        } else {
            format!("{root}.{}", value_column(&cte.anchor.path))
        };
        let hierarchy = &self.config.entity_hierarchy_table;
        let [dt, di, at, ai] = HIERARCHY_COLUMNS;
        Ok(format!(
            "{name} AS (\n  SELECT {root}.{ID_COLUMN} AS {ID_COLUMN}, {} AS \"{at}\", {value} AS \"{ai}\"\n  FROM {root}\n  WHERE {value} IS NOT NULL\n  UNION\n  SELECT anc.{ID_COLUMN}, h.\"{at}\", h.\"{ai}\"\n  FROM {name} AS anc\n  JOIN {hierarchy} AS h ON h.\"{dt}\" = anc.\"{at}\" AND h.\"{di}\" = anc.\"{ai}\"\n)",
            quoted_literal(&cte.ety.to_string())?
        ))
    }

    /// The roots the query selects rows for, in order: the unknown variables.
    pub fn unknown_roots(&self) -> Vec<RootKey> {
        [RootKey::Principal, RootKey::Resource]
            .into_iter()
            .filter(|k| self.roots.contains_key(k))
            .collect()
    }
}

/// A compiled expression that is not a row-anchored entity.
fn plain(sql: String, repr: Repr) -> Compiled {
    Compiled {
        sql,
        repr,
        anchor: None,
    }
}

/// The column every root CTE exports for its entity id.
const ID_COLUMN: &str = "\"$id\"";

/// A JSONB value in the canonical encoding, as a native value of `repr`.
fn json_to_repr(json: &str, repr: &Repr) -> String {
    match repr {
        Repr::Bool => format!("(({json}) #>> '{{}}')::boolean"),
        Repr::Long => format!("(({json}) #>> '{{}}')::bigint"),
        Repr::Text => format!("(({json}) #>> '{{}}')"),
        Repr::Entity(_) => format!("(({json}) -> 'e' ->> 'i')"),
        Repr::Json => format!("({json})"),
    }
}

/// The quoted CTE name of a root.
fn root_name(root: &RootKey) -> String {
    match root {
        RootKey::Principal => "\"principal\"".to_owned(),
        RootKey::Resource => "\"resource\"".to_owned(),
        RootKey::Literal(uid) => shortened(&uid.to_string()).to_string(),
    }
}

fn root_raw_name(root: &RootKey) -> String {
    match root {
        RootKey::Principal => "principal".to_owned(),
        RootKey::Resource => "resource".to_owned(),
        RootKey::Literal(uid) => uid.to_string(),
    }
}

/// The quoted alias of the join for `path` under `root` (the root itself
/// for the empty path).
fn alias_name(root: &RootKey, path: &[SmolStr]) -> String {
    if path.is_empty() {
        return root_name(root);
    }
    shortened(&format!("{}.{}", root_raw_name(root), path.join("."))).to_string()
}

/// The quoted CTE column holding the value of an attribute path.
fn value_column(path: &[SmolStr]) -> String {
    shortened(&format!("$v:{}", path.join("."))).to_string()
}

fn ancestors_name(anchor: &Anchor) -> SQLIdentifier {
    let path = if anchor.path.is_empty() {
        String::new()
    } else {
        format!(".{}", anchor.path.join("."))
    };
    shortened(&format!("{}{path}$ancestors", root_raw_name(&anchor.root)))
}
