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
