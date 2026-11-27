DO $$
BEGIN
 IF (SELECT count(*) FROM finance.job)<>2 OR (SELECT count(*) FROM finance.job WHERE state='SUCCEEDED')<>1 OR (SELECT count(*) FROM finance.job WHERE state='DEAD' AND attempts=3)<>1 THEN RAISE EXCEPTION 'job terminal states mismatch'; END IF;
 IF (SELECT count(*) FROM finance.inbox)<>1 OR (SELECT count(*) FROM finance.job_attempt)<>4 OR (SELECT count(*) FROM finance.job_attempt WHERE state='EXPIRED')<>1 OR (SELECT count(*) FROM finance.job_attempt WHERE state='ACTIVE')<>0 THEN RAISE EXCEPTION 'inbox or attempt mismatch'; END IF;
 IF has_table_privilege('finance_job_worker','finance.inbox','UPDATE') OR has_table_privilege('finance_job_worker','finance.outbox','INSERT') OR has_table_privilege('finance_job_worker','finance.organization_unit','INSERT') OR has_table_privilege('finance_runtime','finance.job','UPDATE') THEN RAISE EXCEPTION 'excess worker/runtime privilege'; END IF;
END $$;
