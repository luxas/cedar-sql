# cedar-sql

**Summary:** This repository contains an experimental Cedar evaluator that works by "compiling" the Cedar policy evaluation and entity fetch process into equivalent SQL. The request environment can be partial, such that e.g. the `principal` is specified but the `resource` is unknown, and this will return a list of `resource` rows that the given `principal` is authorized to access. To begin with, we intend to support Turso and PostgreSQL.

It is worth pointing out that the author of the ideas is Lucas Käldström (@luxas), the ideas are **NOT** LLM-generated, LLMs have just been used as a tool to show something concrete (during the so far short time between Aug 25 - Sept 13 I've had developing these ideas). It should also be mentioned that this `README.md` document is completely written by me, and I consider this document to be the primary contribution in this repo.

Eventually, I intend to improve (most likely by starting more or less from scratch with regards to every feature PR) the code, docs, tests and hopefully add correctness proofs, if the ideas make sense to the Cedar community. I'm happy to contribute the project to cloud native community as well, as wanted/needed. My main goal at this moment is to:

1. validate the ideas by discussing with others (I'm not 100% certain of whether all parts of the ideas make sense)
1. ask for feedback on the direction by the community (feel free to email me about this, or send a DM on the CNCF Slack or LinkedIn!)

> **WARNING:** The commits in this repo **MUST NOT be used in production**; they serve only as a concretization of the ideas presented, to evaluate what direction to evolve the ideas towards, and to give something that can be experimentally tested and iterated on, in search of the final form of the features.

For now, I've only had time to go one pass over this text, I'll try to make it more understandable over time, if needed.
I intend to turn this text into a series of "real" blog posts later, most likely going into more depth on each topic there.

## Why?

When normally performing Cedar authorization, the application which uses Cedar should/must give an appropriate amount of entities data in order for the policies to work. This could be done by e.g.

1. considering any possible row that could be referenced from a policy in the underlying data store (e.g. SQL database) from the current request environment, and getting that. This could be done for each request or through a cache (that needs to be kept up to date), and either taking the current request data (e.g. principal) into account or just fetch the data for any principal,
1. somehow in an application-specific way "knowing" exactly what data needs to be fed, or
1. [batched authorization] could help, at the expense of potentially needing multiple roundtrips to the database

Furthermore, in some cases you don't want/need an `Allow/Deny` decision, but a filter on what entities (usually resources) to return in a request. [Typed Partial Evaluation] (TPE) can help with this, but usually requires some way to transform the returned residual to SQL.

Thus, let's see if we can find a general solution, that works both for the first and second case, and might potentially have competetive performance (this we can only know after trying), at least when comparing to the naive ways that in any case require a db roundtrip.

[batched authorization]: https://docs.rs/cedar-policy/latest/cedar_policy/struct.PolicySet.html#method.is_authorized_batched
[Typed Partial Evaluation]: https://cedarpolicy.com/blog/tpe

## Idea 1: Generate SQL schema from the Cedar schema

SQL is way more expressive than Cedar, and thus is a good starting point to make a sound evaluator to generate the SQL schema from Cedar, with concrete types.
This authorizer thus forces all policies to be successfully validated against the schema before they can be used.

The translation is as follows:

- Each entity type gets its own table `<EntityType>_table` with the `__entity_id` string column serving as the primary key
- Cedar entity type `::` namespace separators are replaced with `_`
- Only simple core types are supported for now:
  - `Bool`
  - `String`
  - `Long`: 64-bit signed integer, like Cedar uses.
  - `EntityUID`: A foreign key to the table of the referenced cedar entity's `__entity_id` column.
  - `Set`: Stored as `[]<SQL type of value>`
  - `Record`: Stored as a `jsonb` column.
- Each attribute maps to a (nullable or non-nullable) column.
- Entity references map into foreign keys to the table of the referenced entity type.
- Tags are handled through their own lookup table `<EntityType>_tags` with columns `entity_id`, `tag`, `value`. `value` has the SQL-mapped type as per the Cedarschema.
- The Cedar `in` DAG hierarchy is stored through an `cedar_entity_hierarchy` table with columns `descendant_type`, `descendant_id`, `ancestor_type`, `ancestor_id`, which stores all parent-child relationships for all entities regardless of type.

The Cedarschema supports annotations that can be used to modify the mappings:

- For entities:
  - `@sql_table("users")`: By default the quoted fully-qualified Cedar entity type name, e.g. `"App::User"`. Maximum 63 characters, if the Cedar entity type name is longer than this, this annotation is required.
  - `@sql_tags_table("users_roles")`: By default the quoted fully-qualified Cedar entity type name, plus a `_tags` suffix. If there is a duplicate conflict with regards to the automatically generated name, this annotation is required (e.g. if there is one Cedar entity `foo` with tags, and another Cedar entity `foo_tags`).
  - `@sql_primary_key("id")`: Defaults to the entity ID column.
  - `@sql_custom_config("{ ... }")`: JSON encoding of the `TableConfiguration` type for specifying custom columns and table modifiers. `TableConfiguration.entity_type` must be `null`/`None`.
  - `@sql_entity_id_column("id")`: Defaults to `__entity_id`. Can point to a SQL column generated by a Cedar attribute, a custom column, or an otherwise undefined name (which results in a new column addition), as long as the target column is of type text, non-nullable and declared unique.
    - TODO: Can we relax the requirement of the column to be `TEXT` by casting in the generation process?
- For entity attributes:
  - `@sql_column("first_name")`: Declares the column name, if different from the Cedarschema attribute name.
  - `@sql_unique("true")`: Declares the column's uniqueness.
  - `@sql_custom_attrs("CHECK first_name <> 'foo'")`: Defines custom column attributes in raw SQL. TODO: Is there any good way to sanitize the SQL to prevent SQL injection-style problems, even though the admin specifying this is trusted.

The program checks that there aren't overlapping column references from the schema (e.g. two distinct Cedar entity attributes `fullName` and `firstName` that map to the same column `name`), with the exception that the column used as the primary key can also be exposed as an attribute.

The default "interface" for a table corresponding to an entity is:

- `__entity_id`: Describes the entity ID.
- `__entity_type`: Used for fully-qualifying the entity ID when computing the transitive closure, where edges might dynamically refer between entity types.

The fact that the SQL condition generator can rely on these entity id/type fields existing in any entity type table leads to simplified and easier to understand SQL queries.

While the above are defaults, a positive integer is appended after each of those names if there are column mappings that conflict, e.g. if there exists `@sql_column("__entity_id")` in one table and `@sql_additional_columns("__entity_id0,__entity_id1")` in another table, `__entity_id2` is chosen as the globally unique column name for representing the entity ID in CTEs (and similar for `__entity_type`). The name for the `cedar_entity_hierarchy` table is similarly computed to be globally unique.

The data model for the database / table / column specification is as follows:

```rust
// Newtype for validated SQL identifiers of max 63 bytes, with proper escaping capabilities.
struct SQLIdentifier(String);

struct DatabaseConfiguration {
  tables: HashMap<SQLIdentifier, TableConfiguration>,
  // cedar_entity_hierarchy or the next available name
  entity_hierarchy_table: SQLIdentifier,
  // Defines the "interface" for all entity tables for specifying the entity ID, even if TableConfiguration.entity_id_column differs between tables.
  entity_id_column: SQLIdentifier,
  // Defines the entity table interface column for the entity type, always statically generated from TableConfiguration.entity_type
  entity_type_column: SQLIdentifier,
}

struct TableConfiguration {
  // Always Some when generated from a Cedarschema.
  // Always None from @sql_custom_config
  entity_type: Option<CedarEntityType>,
  // One or many primary keys.
  primary_keys: HashSet<SQLIdentifier>,
  // 
  columns: HashMap<SQLIdentifier, ColumnConfiguration>,
  tags_table: Option<SQLIdentifier>,
  // This column is always used in Cedar-generated JOINs for entity lookups.
  entity_id_column: SQLIdentifier,
  // TODO: Maybe define some indices here too.
}

enum SQLType {
  Text,              // Cedar: String
  BigInt,            // Cedar: Long
  Bool,              // Cedar: Bool
  Jsonb,             // Cedar: Record
  Set(Arc<SQLType>), // Cedar: Set
  // TODO: UUID as String
  Custom(String),    // Cedar: N/A
}

struct ColumnConfiguration {
  ty: SQLType,
  nullable: bool,
  unique: bool,
  // TODO: Foreign key references defined here
  // TODO: Auto-generated columns for e.g. UUID
  custom_attrs: String,
}
```

```cedarschema
@sql_table("users")
entity User = {
  firstName: String
};

@sql_table("folders")
entity Folder = {
  confidential: Bool
};

@sql_table("documents")
entity Document in [Folder] = {
  parent: Folder
};

action "get" appliesTo {
  principal: [User],
  resource: [Document]
};
```

```sql
CREATE TABLE users (
  __entity_id TEXT NOT NULL UNIQUE
  __entity_type TEXT GENERATED ALWAYS AS 'User'

  firstName TEXT NOT NULL

  PRIMARY KEY (__entity_id)
)

CREATE TABLE folders (
  __entity_id TEXT NOT NULL UNIQUE
  __entity_type TEXT GENERATED ALWAYS AS 'Folder'

  confidential BOOLEAN NOT NULL

  PRIMARY KEY (__entity_id)
)

CREATE TABLE documents (
  __entity_id TEXT NOT NULL UNIQUE
  __entity_type TEXT GENERATED ALWAYS AS 'Document'

  parent TEXT NOT NULL
  
  PRIMARY KEY (__entity_id)
)

CREATE TABLE cedar_entity_hierarchy (
  descendant_type TEXT NOT NULL
  descendant_id TEXT NOT NULL
  ancestor_type TEXT NOT NULL
  ancestor_id TEXT NOT NULL

  PRIMARY KEY (descendant_type, descendant_id, ancestor_type, ancestor_id)
)

--- FKs are added after all table definitions so one doesn't have to care about ordering the table definitions correctly or non-cyclicity.
ALTER TABLE Document_table
ADD CONSTRAINT Document_table_parent_fkey
FOREIGN KEY (parent) REFERENCES Folder_table(__entity_id)
```

A slightly more contrived example that showcases the rewrites would be:

```cedarschema
// Forces the entity hierarchy table to add a "2" suffix.
entity cedar_entity_hierarchy = {
  // Forces a non-"__entity_id" table interface.
  __entity_id: Long,
  // Forces a non-"__entity_type" table interface.
  __entity_type: String,
};

namespace App {
  @sql_entity_id_column("user_id")
  entity User = {
    @sql_unique("true")
    user_id: String,

    // Stored as a jsonb column
    // TODO: Provide a way to also store this info in a separate table?
    // TODO: Add CHECK constraints so data added below this conforms to the schema.
    @sql_column("custom_config")
    config: Config,

    // Forces a non-"__entity_id2" table interface.
    __entity_id2: String,
  } tags UserTag;

  type Config = {
    a: Bool,
    b: String,
  };

  @sql_table("usertags")
  @sql_custom_config("{ ... }")
  // Custom JSON config as follows:
  // {
  //   "primary_keys": ["custom_pk"]
  //   "entity_id_column": "custom_eid",
  //   "columns": {
  //     "custom_pk": {
  //       "ty": "BigInt",
  //       "unique": true,
  //       "custom_attrs": "GENERATED ALWAYS AS IDENTITY"
  //     },
  //     "custom_eid": {
  //       "ty": "Text",
  //       "unique": true,
  //       "custom_attrs": "DEFAULT uuidv7()::text"
  //     }
  //   }
  // }
  entity UserTag = {
    categories?: Set<String>,

    @sql_custom_attrs("DEFAULT TRUE")
    enabled: Bool,
  };
}
```

generates

```sql
CREATE TABLE cedar_entity_hierarchy (
  __entity_id3 TEXT NOT NULL UNIQUE
  __entity_type2 TEXT GENERATED ALWAYS AS 'cedar_entity_hierarchy'

  __entity_id BIGINT NOT NULL
  __entity_type TEXT NOT NULL
  
  PRIMARY KEY (__entity_id3)
)

CREATE TABLE "App::User" (
  __entity_id3 TEXT GENERATED ALWAYS AS user_id
  __entity_type2 TEXT GENERATED ALWAYS AS 'App::User'

  user_id TEXT NOT NULL UNIQUE
  custom_config JSONB NOT NULL
  __entity_id2 TEXT NOT NULL
  
  PRIMARY KEY (user_id)
)

CREATE TABLE "App::User_tags" (
  entity_id TEXT NOT NULL
  tag TEXT NOT NULL

  tag_value TEXT NOT NULL -- Entity ID reference to usertags(custom_eid)
  
  PRIMARY KEY (entity_id, tag)
)

CREATE TABLE usertags (
  __entity_id3 TEXT GENERATED ALWAYS AS custom_eid
  __entity_type2 TEXT GENERATED ALWAYS AS 'Document'

  custom_pk BIGINT NOT NULL UNIQUE GENERATED ALWAYS AS IDENTITY
  custom_eid TEXT NOT NULL DEFAULT uuidv7()::text
  categories []TEXT
  enabled BOOLEAN NOT NULL DEFAULT TRUE
  
  PRIMARY KEY (custom_pk)
)

CREATE TABLE cedar_entity_hierarchy2 (
  descendant_type TEXT NOT NULL
  descendant_id TEXT NOT NULL
  ancestor_type TEXT NOT NULL
  ancestor_id TEXT NOT NULL

  PRIMARY KEY (descendant_type, descendant_id, ancestor_type, ancestor_id)
)

ALTER TABLE "App::User_tags"
ADD CONSTRAINT "App::User_tags_entity_id_fkey"
FOREIGN KEY (entity_id) REFERENCES "App::User"(user_id)

ALTER TABLE "App::User_tags"
ADD CONSTRAINT "App::User_tags_tag_value_fkey"
FOREIGN KEY (tag_value) REFERENCES usertags(custom_eid)
```

## Idea 2: Concrete evaluation is a special case of partial evaluation

The assumption is that SQL database contains all the entity data needed to resolve an authorization decision concretely.
For the part that it does not, the extra data can be fed in in the TPE phase that is executed before we're converting to SQL.
Otherwise, possibly [Postgres foreign data wrappers] could be used.

[Postgres foreign data wrappers]: https://wiki.postgresql.org/wiki/Foreign_data_wrappers

Thus, if there is a mapping from an arbitrary Cedar policy expression to an SQL query, we can concretely authorize a Cedar `PolicySet` by:

1. performing TPE with all variables known (after which the residual policies are variable-free, just possibly refer to Entity UIDs),
1. converting the residual policies to SQL,
1. implementing the `getAttr`, `getTag` and `in` entity data lookups from Postgres, and then
1. combine all individual policy evaluation outcomes into a final `Allow/Deny` decision

The high level expression thus becomes:

```sql
SELECT
  "principal_id",
  "resource_id",
  -- Individual evaluation outcomes for each policy; three-valued true/false/null.
  "a1",
  -- ...
  "aN",
  "d1",
  -- ...
  "dM",
  -- decision is calculated by (a1 = .some true || ... || aN = .some true) AND NOT (d1 = .some true || ... || dM = .some true)
  -- just like how the Cedar authorizer does it
  (
    COALESCE("a1", false) = true || 
    ... || 
    COALESCE("aN", false) = true
  ) AND 
  NOT (
    COALESCE("d1", false) = true || 
    ... || 
    COALESCE("dM", false) = true
  ) AS "decision"
  -- TODO: Allow propagating arbitrary extra columns here as well, although not used in the authz process
FROM (
  WITH RECURSIVE -- recursive needed for the ancestors transitive closure
  -- All entity types must have the __entity_type and __entity_id columns
  "resource" AS (
    SELECT
      "resource"."__entity_id" AS "__entity_id",       -- Always emitted
      "resource"."__entity_type" AS "__entity_type",   -- Always emitted
      "resource.parent"."confidential" AS "parent.confidential"  -- Used in the policy
      -- TODO: Allow propagating arbitrary extra columns here as well, although not used in the authz process
    FROM documents AS "resource"
    -- Entity references must be LEFT JOINed so the base row is not filtered even if the ref doesn't exist
    LEFT JOIN folders AS "resource.parent" ON "resource"."parent" = "resource.parent"."__entity_id"
  ),
  "principal" AS (
    -- from the principal type table
  ),
  "resource_ancestors" AS (
    -- Compute the transitive closure of all ancestors for every possible starting entity (e.g. resource in this case)
  ),
  -- Entity UID references
  "Folder::""foo""" AS (
    SELECT
      "Folder::""foo"""."__entity_id" AS "__entity_id",      -- Always emitted (also referenced in the policy)
      "Folder::""foo"""."__entity_type" AS "__entity_type"   -- Always emitted (also referenced in the policy)
    FROM folders AS "Folder::""foo"""
    WHERE "Folder::""foo"""."__entity_id" = 'foo'
  )
  SELECT
    -- Emitted for the unknown variable (in this case resource, could also be and/or principal)
    "principal"."__entity_id" AS "principal_id",
    "resource"."__entity_id" AS "resource_id",
    (
      -- policy expression
    ) AS policy0
    -- TODO: Allow propagating arbitrary extra columns here as well, although not used in the authz process

  -- (SELECT 1 AS base) is used so that for concrete evaluation (where both principal and resource are substituted for entity UIDs),
  -- there is an authorization decision returned, as the substituted principal / resource entity UIDs might not have existed, but
  -- the query could have completed anyways.
  FROM (SELECT 1 AS base)

  -- However, in the case of partial evaluation, where e.g. the resource is unknown, and you want to return "one row per resource",
  -- using a CROSS JOIN leads to the correct result of zero rows, if the resources table actually has zero rows.
  -- Again, however, if the resource variable was concrete, it is substituted into an Entity UID and thus LEFT JOIN below for those
  CROSS JOIN "resource"
  CROSS JOIN "principal"

  -- Entity UID references must be LEFT JOIN-ed, as they might not exist (i.e. be NULL), and the policy should handle that
  -- without collapsing the number of rows in the original query.
  LEFT JOIN "Folder::""foo""" ON true
  -- ...
)
```

After TPE, the following AST nodes might produce residuals:

### Unknown `principal` or `resource` variable

Depending on whether the `principal` (of `n` rows) and/or `resource` (of `m` rows) variables are unknown, the possible amount of returned rows is `1` (both specified), `n` (`resource` specified), `m` (`principal` specified) or `n*m` (if none specified).

Each referenced attribute is added to the `SELECT` query and every foreign entity reference leads to a new `LEFT JOIN` which adds the ability to select also deeper-nested attributes, e.g.

```sql
WITH
"resource" AS (
  SELECT
    "resource"."__entity_id" AS "__entity_id",       -- Always emitted
    "resource"."__entity_type" AS "__entity_type",   -- Always emitted
    "resource.parent"."confidential" AS "parent.confidential"  -- Used in the policy
    -- TODO: Allow propagating arbitrary extra columns here as well, although not used in the authz process
  FROM documents AS "resource"
  -- Entity references must be LEFT JOINed so the base row is not filtered even if the ref doesn't exist
  LEFT JOIN folders AS "resource.parent" ON "resource"."parent" = "resource.parent"."__entity_id"
),
-- ...
```

### Unknown attributes

If `arg1`'s entity is not in the entity store or `arg1`'s attributes are `None`, `<arg1>.<attr>` and `<arg1> has <attr>` return a residual.

If `<arg1>` already is a root variable or entity UID, the dereference can be just added to the root table's `SELECT` list and readily used in the main query. However, if there is nesting (such as `principal.a.b`), one must make sure that `principal.a` is added through a `LEFT JOIN`, as shown above in the example with `"resource"` (from table `document`) and `"resource.parent"` (from table `folders`).

If `<arg1>` is a record (e.g. `{b: true}`), then it is accessed using `(<expr> ->> '<attr>')::<type>`.
To determine `has`, a null check is done.

Repeated application of `GetAttr`, e.g. `principal.a.b.c && principal.a.b.d.e` yields:

```sql
WITH
"principal" AS (
  SELECT
    "principal"."__entity_id" AS "__entity_id",       -- Always emitted
    "principal"."__entity_type" AS "__entity_type",   -- Always emitted
    "principal.a.b"."c" AS "a.b.c",  -- Used in the policy
    "principal.a.b.d"."e" AS "a.b.d.e",  -- Used in the policy
  FROM <PrincipalType> AS "principal"
  -- Entity references must be LEFT JOINed so the base row is not filtered even if the ref doesn't exist
  LEFT JOIN <PrincipalAType> AS "principal.a" ON "principal"."a" = "principal.a"."__entity_id"
  LEFT JOIN <PrincipalABType> AS "principal.a.b" ON "principal.a"."b" = "principal.a.b"."__entity_id"
  LEFT JOIN <PrincipalABDType> AS "principal.a.b.d" ON "principal.a.b"."d" = "principal.a.b.d"."__entity_id"
),
-- ...
```

and within the `SELECT` policy expression:

```sql
"principal"."a.b.c" AND "principal"."a.b.d.e"
```

(simplified, handling errors will be discussed later)

A couple of observations:

- `<entity>.<attr>` where `<attr>` is of an entity type yields exactly one row, in other words keeps the cardinality
- A bare entity reference such as `User::"foo"` returns exactly one entity (as it naturally filters to one value of a unique row throug `SELECT ... FROM ... WHERE __entity_id = 'foo'`)
- Thus, `LEFT JOIN`ing any amount of `User:"foo".a.b.c...z` just adds extra columns, but doesn't change the amount of rows returned.
- If e.g. the principal entity ID was known during TPE; TPE replaces `principal` with the actual entity ID reference, and then the variable doesn't exist in the expression.

### Unknown tags

If `arg1`'s entity is not in the entity store or `arg1`'s tags are `None`, `<arg1>.getTag(<arg2>)` and `<arg1>.hasTag(<arg2>)` returns a residual.

`<arg1>.getTag(<arg2>)`, e.g. `principal.roles.hasTag(resource.name) && principal.roles.getTag(resource.name) == "bar"` turns into the following within the `SELECT` section:

```sql
WITH
"principal.roles_tags" AS (
  SELECT
    "principal"."__entity_id" AS "__entity_id",       -- Always emitted
    "principal"."__entity_type" AS "__entity_type",   -- Always emitted
    "principal.a.b"."c" AS "a.b.c",  -- Used in the policy
    "principal.a.b.d"."e" AS "a.b.d.e",  -- Used in the policy
  FROM <PrincipalType>_tags AS "principal.roles_tags"
),
-- ...
```

and within the `SELECT` policy expression:

```sql
EXISTS (
  SELECT 1
  FROM <PrincipalRolesType>_tags
  WHERE
    entity_id = "principal"."roles.__entity_id" AND
    tag = "resource"."name"
) AND 
(
  SELECT tag_value
  FROM <PrincipalRolesType>_tags
  WHERE
    entity_id = "principal"."roles.__entity_id" AND
    tag = "resource"."name"
) = 'bar'
```

Nested entities most like `principal.roles.hasTag(resource.name) && principal.roles.getTag(resource.name).a.b` wouldn't probably yield super-performant results (due to the conditional/inline `SELECT`), as then the RHS would compile to something like:

```sql
SELECT
  "principal.roles.getTag(resource.name).a"."b"
FROM (
  SELECT
    "<PrincipalRolesTagValueTypeTable>"."__entity_id" AS __entity_id,
    "<PrincipalRolesTagValueTypeTable>"."__entity_type" AS __entity_type,
    "<PrincipalRolesTagValueTypeTable>"."a" AS "a"
  FROM <PrincipalRolesType>_tags
  INNER JOIN <PrincipalRolesTagValueTypeTable> ON "<PrincipalRolesTagValueTypeTable>"."__entity_id" = "<PrincipalRolesType>_tags"."tag_value"
  WHERE
    entity_id = "principal"."roles.__entity_id" AND
    tag = "resource"."name"
) AS "principal.roles.getTag(resource.name)"
LEFT JOIN <ATypeTable> AS "principal.roles.getTag(resource.name).a" ON "principal.roles.getTag(resource.name)"."a" = "principal.roles.getTag(resource.name).a"."__entity_id"
```

### Unknown ancestors

If `arg1`'s entity is not in the entity store or `arg1`'s ancestors are `None`, `in` returns a residual.

For `residual1 in residual2` to be able to computed, one needs to find the transitive closure of `residual1`'s ancestors like follows:

```sql
WITH RECURSIVE -- needed for the transitive closure CTE
  residual1 AS (<residual1-SQL>),
  -- Note: This would also work even with "resource.parent in XXX"; in that case, columns "parent.__entity_id" and
  -- "parent.__entity_id" would be added to the "resource" CTE, and those would be referenced in the base case.
  "residual1_ancestors" AS (
    -- Base case: "resource in resource" == true
    -- TODO: Add DISTINCT here (if we know the base set is not necessarily unique, e.g. if we'd have "resource.parent in XXX"),
    -- or does that just slow things down?
    SELECT
      "resource"."__entity_id" AS "__entity_id",
      "resource"."__entity_type" AS "__entity_type",
      "resource"."__entity_id" AS "ancestor_id",
      "resource"."__entity_type" AS "ancestor_type"
    FROM "residual1"

    UNION ALL -- TODO: Handle cycles

    SELECT
      "resource_ancestors"."__entity_id",
      "resource_ancestors"."__entity_type",
      "cedar_entity_hierarchy"."ancestor_id",
      "cedar_entity_hierarchy"."ancestor_type"
    FROM "cedar_entity_hierarchy" -- A pre-existing table in the database
    INNER JOIN "resource_ancestors" ON
      "resource_ancestors"."ancestor_type" = "cedar_entity_hierarchy"."descendant_type" AND
      "resource_ancestors"."ancestor_id" = "cedar_entity_hierarchy"."descendant_id"
  ),
```

Then, the check within the `SELECT` becomes:

```sql
EXISTS (
  SELECT 1
  FROM "residual1_ancestors"
  INNER JOIN "residual2" ON
    "residual1_ancestors"."__entity_type" = "residual2".__entity_type AND
    "residual1_ancestors"."__entity_id" = "residual2".__entity_id
)
```

```sql
EXISTS (
  SELECT 1
  FROM "resource_ancestors"
  WHERE
    "residual1_ancestors"."__entity_type" = "residual1"."__entity_type" AND
    "residual1_ancestors"."__entity_id" = "residual1"."__entity_id" AND
    "residual1_ancestors"."ancestor_type" = "residual2"."__entity_type" AND
    "residual1_ancestors"."ancestor_id" = "residual2"."__entity_id"))
```

## Examples

For the sample Cedarschema above:

```cedar
@id("policy0")
permit(
  principal is User,
  action == Action::"get",
  resource is Document in Folder::"foo"
) when {
  principal.firstName == "Lucas" &&
  !resource.parent.confidential
};
```

which after typed partial evaluation in the `(User, Action::"get", Resource)` request environment, for `principal` `User::"luxas"` but with an unknown `resource` reduces to:

```cedar
resource in Folder::"foo" &&
User::"luxas".firstName == "Lucas" &&
!resource.parent.confidential
```

Observe that at relative to a given entity UID or variable root (e.g. `principal` in the expr `principal.foo.bar`), any further dereferences can give exactly one Cedar `Value`. Thus does it make sense to represent all dereferences of a given root variable in the same CTE; this works well even if the root variable could have multiple possible values, e.g. when the `resource` is unknown, any row in the underlying table is selected. But for each such resource instantiation, there is exactly one value corresponding to `resource.a`, `resource.b.c` etc. and thus are those represented as columns.

Note: Only those attributes that are referenced are `SELECT`ed by the CTE.

Evaluated for the `principal` `User::"luxas"` but an unknown `resource`:

```sql
SELECT
  "resource_id",
  -- Individual evaluation outcomes for each policy; three-valued true/false/null.
  "policy0",
  -- decision is calculated by (a1 = .some true || ... || aN = .some true) AND NOT (d1 = .some true || ... || dM = .some true)
  -- just like how the Cedar authorizer does it
  COALESCE("policy0", false) = true AS "decision"
  -- TODO: Allow propagating arbitrary extra columns here as well, although not used in the authz process
FROM (
  WITH RECURSIVE -- recursive needed for the ancestors transitive closure
  -- All entity types must have the __entity_type and __entity_id columns
  "resource" AS (
    SELECT
      "resource"."__entity_id" AS "__entity_id",       -- Always emitted
      "resource"."__entity_type" AS "__entity_type",   -- Always emitted
      "resource.parent"."confidential" AS "parent.confidential"  -- Used in the policy
      -- TODO: Allow propagating arbitrary extra columns here as well, although not used in the authz process
    FROM documents AS "resource"
    -- Entity references must be LEFT JOINed so the base row is not filtered even if the ref doesn't exist
    LEFT JOIN folders AS "resource.parent" ON "resource"."parent" = "resource.parent"."__entity_id"
  ),
  -- Note: This would also work even with "resource.parent in XXX"; in that case, columns "parent.__entity_id" and
  -- "parent.__entity_id" would be added to the "resource" CTE, and those would be referenced in the base case.
  "resource_ancestors" AS (
    -- Base case: "resource in resource" == true
    -- TODO: Add DISTINCT here (if we know the base set is not necessarily unique, e.g. if we'd have "resource.parent in XXX"),
    -- or does that just slow things down?
    SELECT
      "resource"."__entity_id" AS "__entity_id",
      "resource"."__entity_type" AS "__entity_type",
      "resource"."__entity_id" AS "ancestor_id",
      "resource"."__entity_type" AS "ancestor_type"
    FROM "resource"

    UNION ALL -- TODO: Handle cycles

    SELECT
      "resource_ancestors"."__entity_id",
      "resource_ancestors"."__entity_type",
      "cedar_entity_hierarchy"."ancestor_id",
      "cedar_entity_hierarchy"."ancestor_type"
    FROM "cedar_entity_hierarchy" -- A pre-existing table in the database
    INNER JOIN "resource_ancestors" ON
      "resource_ancestors"."ancestor_type" = "cedar_entity_hierarchy"."descendant_type" AND
      "resource_ancestors"."ancestor_id" = "cedar_entity_hierarchy"."descendant_id"
  ),
  "Folder::""foo""" AS (
    SELECT
      "Folder::""foo"""."__entity_id" AS "__entity_id",      -- Always emitted (also referenced in the policy)
      "Folder::""foo"""."__entity_type" AS "__entity_type"   -- Always emitted (also referenced in the policy)
    FROM folders AS "Folder::""foo"""
    WHERE "Folder::""foo"""."__entity_id" = 'foo'
  ),
  "User::""luxas""" AS (
    SELECT
      "User::""luxas"""."__entity_id" AS "__entity_id",      -- Always emitted
      "User::""luxas"""."__entity_type" AS "__entity_type",  -- Always emitted
      "User::""luxas"""."firstName" AS "firstName"           -- Referenced in the policy, and thus emitted
    FROM users AS "User::""luxas"""
    WHERE "User::""luxas"""."__entity_id" = 'luxas'
  )
  SELECT
    -- Emitted for the unknown variable (in this case resource, could also be and/or principal)
    "resource"."__entity_id" AS "resource_id",
    (
      (EXISTS (SELECT 1 FROM "resource_ancestors" WHERE
        "resource_ancestors"."__entity_type" = "resource"."__entity_type" AND
        "resource_ancestors"."__entity_id" = "resource"."__entity_id" AND
        "resource_ancestors"."ancestor_type" = "Folder::""foo"""."__entity_type" AND
        "resource_ancestors"."ancestor_id" = "Folder::""foo"""."__entity_id")) AND
      ("User::""luxas"""."firstName" = 'Lucas') AND
      (NOT ("resource"."parent.confidential"))
    ) AS policy0
    -- TODO: Allow propagating arbitrary extra columns here as well, although not used in the authz process

  -- (SELECT 1 AS base) is used so that for concrete evaluation (where both principal and resource are substituted for entity UIDs),
  -- there is an authorization decision returned, as the substituted principal / resource entity UIDs might not have existed, but
  -- the query could have completed anyways.
  FROM (SELECT 1 AS base)

  -- However, in the case of partial evaluation, where e.g. the resource is unknown, and you want to return "one row per resource",
  -- using a CROSS JOIN leads to the correct result of zero rows, if the resources table actually has zero rows.
  -- Again, however, if the resource variable was concrete, it is substituted into an Entity UID and thus LEFT JOIN below for those
  CROSS JOIN "resource"

  -- Entity UID references must be LEFT JOIN-ed, as they might not exist (i.e. be NULL), and the policy should handle that
  -- without collapsing the number of rows in the original query.
  LEFT JOIN "Folder::""foo""" ON true
  LEFT JOIN "User::""luxas""" ON true
)
```

## Error handling

An error is denoted by `NULL`, but since `NULL AND false` is `false` in normal Kleene logic, and the database otherwise is free to reorder evaluation terms, `a && b` becomes `CASE WHEN a THEN b ELSE a END`, which keeps the short-circuiting behavior of `a` when it errors (with the downside it becomes duplicated).

`a || b` thus becomes `CASE WHEN (NOT a) THEN b ELSE a END` and `if a then b else c` becomes `CASE WHEN a THEN b ELSE (CASE WHEN (NOT a) THEN c ELSE a END) END`.
Other operators are handled accordingly.

## Testing

The evaluator should eventually pass all Cedar evaluator test using the DRT framework in the `cedar-spec` repo, as well as the `cedar-integration-tests` bundle.

## Status and phases

The implementation is built in phases, one branch and plan document each (`docs/plans/N-slug.md` here, with the
differential tests and their `docs/plans/cedar-sql-N-slug.md` in the [`cedar-sql-spec`] fork of `cedar-spec`):

1. **init** — the crate skeleton, the Postgres provisioning for tests, CI.
1. **schema** — Idea 1: the schema annotations, the database configuration, DDL, and loading Cedar entities as rows.
1. **is-authorized** — Idea 2 for a concrete request over the core operators, and the first differential test.
1. **values** — sets, records, `like`, tags, and the remaining operators.
1. **query** — unknown `principal` and/or `resource`, returning one row per candidate.
1. **corpus** — the `cedar-integration-tests` corpus and hardening.
1. **extensions** — the Cedar extension types.
1. **sqlite** — SQLite and Turso.
1. **benchmarks**.

Deviations from the text above that were decided during implementation:

- Sets and records are stored as canonical JSONB (sets deduplicated and sorted, records and entity references
  wrapped) rather than `[]<type>` arrays, so that JSONB equality is Cedar equality for every nested shape.
- The hierarchy table may hold the transitive closure instead of the direct edges (the recursive CTE tolerates both).
- The `@sql_entity_id_column` target may be the table's (sole) primary key instead of being declared unique.
- Foreign keys are `DEFERRABLE INITIALLY DEFERRED`, so rows load in any order within a transaction.

The crate expects a checkout of [`cedar-woodpecker`] (a fork of `cedar`) next to this repository, as `cedar-spec`
does. Tests need a Postgres: set `CEDAR_SQL_PG_URL`, or leave it unset to have an embedded Postgres downloaded and
started on first use.

[`cedar-sql-spec`]: https://github.com/luxas/cedar-sql-spec
[`cedar-woodpecker`]: https://github.com/luxas/cedar-woodpecker

## Benchmarks

- Compare how fast it is for single checks vs "cross-product of principal x resource"
- Measure CPU/memory as it scales
- Check scalability of `arg1 in [arg2, arg3, ...]` as the list grows.
- Check how fast this evaluator is, compared to
  - Naively reading all entities from Postgres into Cedar `Entities` and first then doing concrete authorization
  - Doing batched authorization, always getting the missing entity refs (i.e. potentially doing multiple db roundtrips) as a function of depth
  - Doing TPE, then getting all possible `principal` and/or `resource` entities from the db, and running concrete authorization over each pair.

## Notes

The Cedar typechecker enforces that each residual expression has a well-known type. For example, the following two examples are rejected by the typechecker. Validating

```cedar
// This doesn't work even though for the purpose of illustration,
// both the User and Document type would have the "id" attribute 
(if principal.isAdmin then User::"alice" else Document::"doc") has id
```

yields

```
the types User and Document are not compatible
help: for policy `test`, both branches of a conditional must have compatible types. Different entity types are never compatible even when their attributes would be compatible
```

and

```cedar
(if principal.isAdmin then {a: "foo", s: "bla"} else {s: ""}) has s
```

yields

```
the types {a: String,s: String,} and {s: String,} are not compatible
help: for policy `test`, both branches of a conditional must have compatible types. Compatible record types must have exactly the same attributes
```

## Limitations and Future work

- **Fixed primary key column**: In the future, make the `__entity_id` column name configurable.
- **No extension types supported**: Cedar decimal, ip address, timestamp, etc. types are left as future work.
- **Untyped record attributes**: Record attribute values are stored as a JSON column, that the content in the row actually conforms to the SQL schema is not enforced by the framework at the moment.
- **Improved entity type name escaping**: Right now, a Cedar entity type `foo::name` and `foo_name` both turn into the `foo_name` table name. Cedar entity type `foo` gets table `foo_tags`, which could conflict with entity type `foo_tags`.
- **Untyped ancestors table**: Consider whether it makes sense to synthesize many typed tables instead, it probably makes (potentially more) sense, especially if the user already has or wants to control specific many-to-many tables.
- **In concrete mode, is it possible for two rows to be returned?**: There shouldn't be, thanks to how primary keys are set up in the schema, but this should be verified.
- **Partial context support**: For now, this is out of scope, as it doesn't relate to the database.
- **Policy expression memoization**: For example, `arg1.hasTag(arg2) && arg1.getTag(arg2)` could yield unneccessary expansion of `arg1` and `arg2` twice, unless memoized/de-duplicated.
- **Custom column names in SQL**: Right now, it is assumed there is an exact match between
- **Other databases than Postgres 18 or Turso**: This can come later, but right now to make things easier, that is the assumption.

## Copyright

Lucas Käldström 2026

## License

Apache 2.0
