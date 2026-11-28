# Registered schema closure

Migration 22 creates the 28 table entities that were present in the v2 ownership registry but absent from executable DDL. Runtime receives no new grants in this migration; RLS is enabled and forced on every new table, and append-only evidence/effect tables have mutation-reject triggers. The registry validator now requires exact equality between all registered table names and all executable `CREATE TABLE finance.*` statements.

Inventory after this migration: 70 registered entities, 70 executable tables, 24 owners.
