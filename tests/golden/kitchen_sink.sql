CREATE TABLE "Doc" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT GENERATED ALWAYS AS ('Doc') STORED,
  "owner" TEXT NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "Doc_tags" (
  "entity_id" TEXT NOT NULL,
  "tag" TEXT NOT NULL,
  "value" TEXT NOT NULL,
  PRIMARY KEY ("entity_id", "tag")
);
CREATE TABLE "Group" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT GENERATED ALWAYS AS ('Group') STORED,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "User" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT GENERATED ALWAYS AS ('User') STORED,
  "admin" BOOLEAN NOT NULL,
  "age" BIGINT,
  "friend" TEXT,
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
