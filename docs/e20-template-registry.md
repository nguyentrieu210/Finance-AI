# E20 template registry

Template aggregate IDs are stable across immutable version rows. Create/revise are PolicyAdmin-only and derive SHA-256 from a CLEAN document blob inside PostgreSQL; the client cannot supply a digest. Format must match the stored MIME. DOCX/XLSX source containing the macro marker vbaProject.bin is rejected before registration. Publication is coupled to the policy maker/checker transition: only the newest draft per template family becomes PUBLISHED and older drafts retire. Requesters can read only PUBLISHED templates.
