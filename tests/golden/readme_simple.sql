CREATE TABLE "documents" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT GENERATED ALWAYS AS ('Document') STORED,
  "parent" TEXT NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "folders" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT GENERATED ALWAYS AS ('Folder') STORED,
  "confidential" BOOLEAN NOT NULL,
  PRIMARY KEY ("__entity_id")
);
CREATE TABLE "users" (
  "__entity_id" TEXT NOT NULL,
  "__entity_type" TEXT GENERATED ALWAYS AS ('User') STORED,
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
