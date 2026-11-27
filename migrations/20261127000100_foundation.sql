-- A00 baseline only; migration owner executes, never application/worker.
CREATE SCHEMA finance;
REVOKE ALL ON SCHEMA finance FROM PUBLIC;
CREATE EXTENSION vector WITH SCHEMA finance;
-- No dossier, audit, membership or payment tables yet.
