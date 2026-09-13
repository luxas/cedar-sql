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
CREATE TABLE "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaääääääää" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Doc') STORED,
  "owner" TEXT NOT NULL,
  "team" TEXT NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaääääää_tags" (
  "entity_id" TEXT NOT NULL,
  "tag" TEXT NOT NULL,
  "value" TEXT NOT NULL,
  PRIMARY KEY ("entity_id", "tag")
);
CREATE TABLE "Team" (
  "__entity_id" TEXT NOT NULL GENERATED ALWAYS AS ("external_id") STORED,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Team') STORED,
  "name" TEXT NOT NULL,
  "external_id" TEXT NOT NULL UNIQUE,
  PRIMARY KEY ("external_id")
);
CREATE TABLE "User" (
  "__entity_id" TEXT NOT NULL UNIQUE,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('User') STORED,
  "id" TEXT NOT NULL,
  PRIMARY KEY ("id")
);
CREATE TABLE "cedar_entity_hierarchy" (
  "descendant_type" TEXT NOT NULL,
  "descendant_id" TEXT NOT NULL,
  "ancestor_type" TEXT NOT NULL,
  "ancestor_id" TEXT NOT NULL,
  PRIMARY KEY ("descendant_type", "descendant_id", "ancestor_type", "ancestor_id")
);
ALTER TABLE "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaääääääää" ADD CONSTRAINT "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa_f207ae3e3534e7d4" FOREIGN KEY ("owner") REFERENCES "User" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaääääääää" ADD CONSTRAINT "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa_49a8c18fc7e9392c" FOREIGN KEY ("team") REFERENCES "Team" ("external_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaääääää_tags" ADD CONSTRAINT "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaä_13084d4062a315cf" FOREIGN KEY ("entity_id") REFERENCES "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaääääääää" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaääääää_tags" ADD CONSTRAINT "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaä_37068e80ddbc8e6f" FOREIGN KEY ("value") REFERENCES "User" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
