CREATE OR REPLACE FUNCTION cedar_eq(a jsonb, b jsonb) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE STRICT AS $cedar$
BEGIN
  IF jsonb_typeof(a) = 'array' AND jsonb_typeof(b) = 'array' THEN
    RETURN NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(a) AS x
        WHERE NOT EXISTS (SELECT 1 FROM jsonb_array_elements(b) AS y WHERE cedar_eq(x.value, y.value)))
      AND NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(b) AS y
        WHERE NOT EXISTS (SELECT 1 FROM jsonb_array_elements(a) AS x WHERE cedar_eq(x.value, y.value)));
  ELSIF jsonb_typeof(a) = 'object' AND jsonb_typeof(b) = 'object' THEN
    RETURN (SELECT coalesce(array_agg(k ORDER BY k), '{}') FROM jsonb_object_keys(a) AS k)
         = (SELECT coalesce(array_agg(k ORDER BY k), '{}') FROM jsonb_object_keys(b) AS k)
      AND NOT EXISTS (
        SELECT 1 FROM jsonb_each(a) AS e WHERE NOT cedar_eq(e.value, b -> e.key));
  ELSE
    RETURN a = b;
  END IF;
END
$cedar$;
CREATE TABLE "documents" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Document') STORED,
  "parent" TEXT NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "folders" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Folder') STORED,
  "confidential" BOOLEAN NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "users" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('User') STORED,
  "firstName" TEXT NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "cedar_entity_hierarchy" (
  "descendant_type" TEXT NOT NULL,
  "descendant_id" TEXT NOT NULL,
  "ancestor_type" TEXT NOT NULL,
  "ancestor_id" TEXT NOT NULL,
  PRIMARY KEY ("descendant_type", "descendant_id", "ancestor_type", "ancestor_id")
);
ALTER TABLE "documents" ADD CONSTRAINT "documents_parent_fkey" FOREIGN KEY ("parent") REFERENCES "folders" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
