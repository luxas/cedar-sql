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
CREATE TABLE "Color" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Color') STORED,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "Doc" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Doc') STORED,
  "owner" TEXT NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "Doc_tags" (
  "entity_id" TEXT NOT NULL,
  "tag" TEXT NOT NULL,
  "value" TEXT NOT NULL,
  PRIMARY KEY ("entity_id", "tag")
);
CREATE TABLE "Empty" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Empty') STORED,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "Group" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('Group') STORED,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "User" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT NOT NULL GENERATED ALWAYS AS ('User') STORED,
  "admin" BOOLEAN NOT NULL,
  "age" BIGINT,
  "friend" TEXT,
  "friend.name" TEXT,
  "friends" JSONB NOT NULL,
  "groups" JSONB NOT NULL,
  "name" TEXT NOT NULL,
  "profile" JSONB NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "User_tags" (
  "entity_id" TEXT NOT NULL,
  "tag" TEXT NOT NULL,
  "value" TEXT NOT NULL,
  PRIMARY KEY ("entity_id", "tag")
);
CREATE TABLE "cedar_entity_hierarchy" (
  "descendant_type" TEXT NOT NULL,
  "descendant_id" TEXT NOT NULL,
  "ancestor_type" TEXT NOT NULL,
  "ancestor_id" TEXT NOT NULL,
  PRIMARY KEY ("descendant_type", "descendant_id", "ancestor_type", "ancestor_id")
);
ALTER TABLE "Doc" ADD CONSTRAINT "Doc_owner_fkey" FOREIGN KEY ("owner") REFERENCES "User" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "Doc_tags" ADD CONSTRAINT "Doc_tags_entity_id_fkey" FOREIGN KEY ("entity_id") REFERENCES "Doc" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "Doc_tags" ADD CONSTRAINT "Doc_tags_value_fkey" FOREIGN KEY ("value") REFERENCES "User" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "User" ADD CONSTRAINT "User_friend_fkey" FOREIGN KEY ("friend") REFERENCES "User" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE "User_tags" ADD CONSTRAINT "User_tags_entity_id_fkey" FOREIGN KEY ("entity_id") REFERENCES "User" ("__entity_id") DEFERRABLE INITIALLY DEFERRED;
