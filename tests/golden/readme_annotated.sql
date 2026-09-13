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
CREATE TABLE "App::User" (
  "__entity_id3" TEXT NOT NULL GENERATED ALWAYS AS ("user_id") STORED,
  "__entity_type2" TEXT NOT NULL GENERATED ALWAYS AS ('App::User') STORED,
  "__entity_id2" TEXT NOT NULL,
  "custom_config" JSONB NOT NULL,
  "user_id" TEXT NOT NULL UNIQUE,
  PRIMARY KEY ("user_id")
);
CREATE TABLE "App::User_tags" (
  "entity_id" TEXT NOT NULL,
  "tag" TEXT NOT NULL,
  "value" TEXT NOT NULL,
  PRIMARY KEY ("entity_id", "tag")
);
CREATE TABLE "usertags" (
  "__entity_id3" TEXT NOT NULL GENERATED ALWAYS AS ("custom_eid") STORED,
  "__entity_type2" TEXT NOT NULL GENERATED ALWAYS AS ('App::UserTag') STORED,
  "categories" JSONB,
  "enabled" BOOLEAN NOT NULL DEFAULT TRUE,
  "custom_pk" BIGINT NOT NULL UNIQUE GENERATED ALWAYS AS IDENTITY,
  "custom_eid" TEXT NOT NULL UNIQUE DEFAULT uuidv7()::text,
  PRIMARY KEY ("custom_pk")
);
CREATE TABLE "cedar_entity_hierarchy" (
  "__entity_id3" TEXT NOT NULL,
  "__entity_type2" TEXT NOT NULL GENERATED ALWAYS AS ('cedar_entity_hierarchy') STORED,
  "__entity_id" BIGINT NOT NULL,
  "__entity_type" TEXT NOT NULL,
  PRIMARY KEY ("__entity_id3")
);
CREATE TABLE "cedar_entity_hierarchy2" (
  "descendant_type" TEXT NOT NULL,
  "descendant_id" TEXT NOT NULL,
  "ancestor_type" TEXT NOT NULL,
  "ancestor_id" TEXT NOT NULL,
  PRIMARY KEY ("descendant_type", "descendant_id", "ancestor_type", "ancestor_id")
);
ALTER TABLE "App::User_tags" ADD CONSTRAINT "App::User_tags_entity_id_fkey" FOREIGN KEY ("entity_id") REFERENCES "App::User" ("user_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "App::User_tags" ADD CONSTRAINT "App::User_tags_value_fkey" FOREIGN KEY ("value") REFERENCES "usertags" ("custom_eid") DEFERRABLE INITIALLY DEFERRED;
