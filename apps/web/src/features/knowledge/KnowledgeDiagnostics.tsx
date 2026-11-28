import { useEffect, useRef, useState } from 'react';

type Hit = {
  chunk_id: string;
  policy_id: string;
  clause_id: string;
  excerpt: string;
  source_digest: string;
  source_page: number;
  distance: number;
};

export function KnowledgeDiagnostics() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [query, setQuery] = useState('settlement');
  const [hits, setHits] = useState<Hit[]>([]);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('Chưa kết nối.');
  const credential = useRef('');
  const abort = useRef<AbortController | null>(null);
  useEffect(() => () => { abort.current?.abort(); credential.current = ''; }, []);

  async function search() {
    if (!credential.current || busy) return;
    abort.current?.abort();
    const controller = new AbortController();
    abort.current = controller;
    setBusy(true);
    setStatus('Đang chạy retrieval trên policy PUBLISHED...');
    try {
      const params = new URLSearchParams({
        kind: 'SETTLEMENT',
        day: '2031-06-01',
        q: query,
        limit: '10',
      });
      const response = await fetch('/__diagnostics/knowledge?' + params, {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
        signal: controller.signal,
      });
      if (!response.ok) {
        if (response.status === 401 || response.status === 403) {
          credential.current = '';
          setConnected(false);
        }
        throw new Error(String(response.status));
      }
      const data = await response.json() as Hit[];
      if (!Array.isArray(data)) throw new Error('invalid response');
      setHits(data);
      setConnected(true);
      setStatus('Kết quả là citation từ clause đã công bố; không phải quyết định AI.');
    } catch {
      if (!controller.signal.aborted) {
        setHits([]);
        setStatus('Không chạy được retrieval.');
      }
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  }

  function clear() {
    abort.current?.abort();
    credential.current = '';
    setToken('');
    setConnected(false);
    setHits([]);
    setBusy(false);
    setStatus('Đã ngắt.');
  }

  return <section aria-labelledby="knowledge-title">
    <h2 id="knowledge-title">Tri thức — chẩn đoán nội bộ</h2>
    <p>Lexical + pgvector trên clause PUBLISHED, synthetic model 3 chiều. Không phát hành policy hoặc rule.</p>
    <label htmlFor="knowledge-token">Token Requester sandbox</label>
    <div className="actions">
      <input id="knowledge-token" type="password" autoComplete="off" maxLength={16384} value={token} disabled={busy || connected} onChange={e => setToken(e.target.value)} />
      <button className="primary" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); void search(); }}>Kết nối</button>
      <button disabled={busy} onClick={clear}>Ngắt</button>
    </div>
    <label htmlFor="knowledge-query">Từ khóa synthetic</label>
    <div className="actions">
      <input id="knowledge-query" maxLength={256} value={query} disabled={!connected || busy} onChange={e => setQuery(e.target.value)} />
      <button disabled={!connected || busy || !query.trim()} onClick={() => void search()}>Tìm</button>
    </div>
    <p role="status">{status}</p>
    <div className="table-wrap"><table>
      <thead><tr><th>Clause</th><th>Trang</th><th>Khoảng cách</th><th>Trích dẫn</th></tr></thead>
      <tbody>{hits.map(hit => <tr key={hit.chunk_id}><td>{hit.clause_id}</td><td>{hit.source_page}</td><td>{hit.distance.toFixed(4)}</td><td>{hit.excerpt}</td></tr>)}</tbody>
    </table></div>
  </section>;
}
