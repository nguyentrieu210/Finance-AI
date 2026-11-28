-- E13 verified advance issue metadata. Existing imported history may lack
-- purpose, so the constraint is NOT VALID: it is enforced for all new/updated
-- rows without rewriting or falsely blessing legacy snapshots.
ALTER TABLE finance.advance_ref ADD COLUMN purpose text;
ALTER TABLE finance.advance_ref ADD CONSTRAINT advance_ref_purpose_required
 CHECK(purpose IS NOT NULL AND octet_length(purpose) BETWEEN 1 AND 512) NOT VALID;
