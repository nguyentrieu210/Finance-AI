import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Membership = components['schemas']['Membership'];

type JwtPreview = { sub?: string; iss?: string; aud?: string | string[]; exp?: number };

function decodeJwtClaims(raw: string): JwtPreview | null {
  const parts = raw.split('.');
  if (parts.length !== 3) return null;
  try {
    const base64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const json = atob(base64);
    return JSON.parse(json) as JwtPreview;
  } catch {
    return null;
  }
}

export function IdentityWorkbench() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [claims, setClaims] = useState<JwtPreview | null>(null);
  const [subjectId, setSubjectId] = useState('');
  const [membership, setMembership] = useState<Membership | null>(null);
  const [status, setStatus] = useState('Chưa kết nối.');
  const [busy, setBusy] = useState(false);
  const credential = useRef('');

  useEffect(() => () => {
    credential.current = '';
  }, []);

  function disconnect() {
    credential.current = '';
    setToken('');
    setConnected(false);
    setClaims(null);
    setMembership(null);
    setStatus('Đã ngắt kết nối.');
  }

  async function verifySession() {
    if (!credential.current) return;
    setBusy(true);
    setStatus('Đang xác thực danh tính qua API...');
    try {
      const res = await fetch('/v1/dossiers?limit=1', {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      if (res.status === 401) {
        credential.current = '';
        setConnected(false);
        setStatus('Token không hợp lệ hoặc đã hết hạn (401).');
        return;
      }
      setStatus(`Token được cổng xác thực chấp nhận (HTTP ${res.status}).`);
    } catch {
      setStatus('Không kết nối được API danh tính.');
    } finally {
      setBusy(false);
    }
  }

  async function lookupSubject() {
    const id = subjectId.trim();
    if (!credential.current || !id) return;
    setBusy(true);
    setStatus('Đang tra cứu subject...');
    try {
      const res = await fetch('/v1/memberships/' + encodeURIComponent(id), {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        setMembership(null);
        setStatus(`Không đọc được subject (${res.status}).`);
        return;
      }
      const data = await res.json() as Membership;
      setMembership(data);
      setStatus(`Subject ${data.subject_id} (${data.active ? 'ACTIVE' : 'REVOKED'}).`);
    } catch {
      setStatus('Lỗi mạng khi đọc membership.');
    } finally {
      setBusy(false);
    }
  }

  return <section aria-labelledby="e02-title">
    <h2 id="e02-title">Danh tính &amp; Phiên làm việc (E02)</h2>
    <div className="sandbox-connection">
      <label htmlFor="e02-token">Token sandbox</label>
      <div className="actions">
        <input id="e02-token" type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} disabled={busy || connected} maxLength={16384} />
        <button className="primary" type="button" disabled={busy || connected || !token.trim()} onClick={() => {
          const clean = token.trim();
          credential.current = clean;
          setToken('');
          setClaims(decodeJwtClaims(clean));
          setConnected(true);
          void verifySession();
        }}>Kết nối</button>
        <button type="button" onClick={disconnect}>Ngắt kết nối</button>
      </div>
    </div>
    <p role="status" aria-live="polite">{status}</p>
    {claims && <div className="table-wrap"><table><thead><tr><th>Subject</th><th>Issuer</th><th>Audience</th><th>Exp</th></tr></thead>
      <tbody><tr><td>{claims.sub ?? '-'}</td><td>{claims.iss ?? '-'}</td><td>{Array.isArray(claims.aud) ? claims.aud.join(', ') : (claims.aud ?? '-')}</td><td>{claims.exp ?? '-'}</td></tr></tbody>
    </table></div>}
    <div className="actions">
      <label htmlFor="e02-subject">Subject UUID</label>
      <input id="e02-subject" value={subjectId} onChange={e => setSubjectId(e.target.value)} placeholder="00000000-0000-0000-0000-0000000003ea" />
      <button type="button" disabled={!connected || busy || !subjectId.trim()} onClick={() => void lookupSubject()}>Kiểm tra Subject</button>
    </div>
    {membership && <p>Vai trò hiệu lực: {membership.roles.join(', ')} (Công ty {membership.company_id})</p>}
  </section>;
}
