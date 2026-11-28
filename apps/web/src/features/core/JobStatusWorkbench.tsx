import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Job = components['schemas']['Job'];

export function JobStatusWorkbench() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [health, setHealth] = useState('Chưa kiểm tra.');
  const [jobId, setJobId] = useState('');
  const [job, setJob] = useState<Job | null>(null);
  const [status, setStatus] = useState('Chưa kết nối.');
  const [busy, setBusy] = useState(false);
  const credential = useRef('');
  const pending = useRef<AbortController | null>(null);

  useEffect(() => () => {
    pending.current?.abort();
    credential.current = '';
  }, []);

  function disconnect() {
    pending.current?.abort();
    credential.current = '';
    setToken('');
    setConnected(false);
    setJob(null);
    setStatus('Đã ngắt kết nối.');
  }

  async function checkHealth() {
    setBusy(true);
    try {
      const [live, ready] = await Promise.all([
        fetch('/health/live', { cache: 'no-store', credentials: 'omit' }),
        fetch('/health/ready', { cache: 'no-store', credentials: 'omit' }),
      ]);
      setHealth(`live=${live.status}, ready=${ready.status}`);
    } catch {
      setHealth('Không kiểm tra được health/ready.');
    } finally {
      setBusy(false);
    }
  }

  async function lookupJob() {
    const id = jobId.trim();
    if (!credential.current || !id) return;
    pending.current?.abort();
    const ac = new AbortController();
    pending.current = ac;
    setBusy(true);
    setStatus('Đang truy vấn tiến trình xử lý...');
    try {
      const res = await fetch('/v1/jobs/' + encodeURIComponent(id), {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
        signal: ac.signal,
      });
      if (!res.ok) {
        if (res.status === 401 || res.status === 403) {
          credential.current = '';
          setConnected(false);
        }
        setJob(null);
        setStatus(`Không tải được job (${res.status}).`);
        return;
      }
      const data = await res.json() as Job;
      setJob(data);
      setStatus(`Trạng thái job: ${data.state} (${data.progress}%).`);
    } catch {
      if (!ac.signal.aborted) setStatus('Lỗi kết nối khi đọc /v1/jobs.');
    } finally {
      setBusy(false);
    }
  }

  return <section aria-labelledby="e01-title">
    <h2 id="e01-title">Core &amp; Trạng thái Job (E01)</h2>
    <div className="sandbox-connection">
      <label htmlFor="e01-token">Token sandbox</label>
      <div className="actions">
        <input id="e01-token" type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} disabled={busy || connected} maxLength={16384} />
        <button className="primary" type="button" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); setConnected(true); setStatus('Đã nạp token.'); void checkHealth(); }}>Kết nối</button>
        <button type="button" onClick={disconnect}>Ngắt kết nối</button>
        <button type="button" disabled={busy} onClick={() => void checkHealth()}>Kiểm tra Health</button>
      </div>
    </div>
    <p>Hệ thống: {health}</p>
    <div className="actions">
      <label htmlFor="e01-job-id">Job UUID</label>
      <input id="e01-job-id" value={jobId} onChange={e => setJobId(e.target.value)} placeholder="00000000-0000-0000-0000-000000000000" />
      <button type="button" disabled={!connected || busy || !jobId.trim()} onClick={() => void lookupJob()}>Tra cứu Job</button>
    </div>
    <p role="status" aria-live="polite">{status}</p>
    {job && <div className="table-wrap"><table><thead><tr><th>ID</th><th>State</th><th>Attempt</th><th>Progress</th><th>Error</th></tr></thead>
      <tbody><tr><td>{job.id}</td><td>{job.state}</td><td>{job.attempt}</td><td>{job.progress}%</td><td>{job.error_code ?? '-'}</td></tr></tbody>
    </table></div>}
  </section>;
}
