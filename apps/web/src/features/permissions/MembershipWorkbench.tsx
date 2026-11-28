import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Membership = components['schemas']['Membership'];
type Page = { items: Membership[]; next_cursor?: string };

export function MembershipWorkbench() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [items, setItems] = useState<Membership[]>([]);
  const [cursor, setCursor] = useState<string | undefined>();
  const [subjectId, setSubjectId] = useState('');
  const [companyId, setCompanyId] = useState('00000000-0000-0000-0000-00000000000a');
  const [rolesText, setRolesText] = useState('REQUESTER');
  const [revokeReason, setRevokeReason] = useState('Thu hồi quyền truy cập sandbox');
  const [status, setStatus] = useState('Chưa kết nối OrgAdmin.');
  const [busy, setBusy] = useState(false);
  const credential = useRef('');

  useEffect(() => () => {
    credential.current = '';
  }, []);

  function disconnect() {
    credential.current = '';
    setToken('');
    setConnected(false);
    setItems([]);
    setCursor(undefined);
    setStatus('Đã ngắt kết nối.');
  }

  async function load(next?: string) {
    if (!credential.current) return;
    setBusy(true);
    setStatus('Đang tải danh sách phân quyền...');
    try {
      const q = new URLSearchParams({ limit: '20' });
      if (next) q.set('cursor', next);
      const res = await fetch('/v1/memberships?' + q, {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        if (res.status === 401 || res.status === 403) {
          credential.current = '';
          setConnected(false);
        }
        setItems([]);
        setStatus(`Không tải được memberships (${res.status}).`);
        return;
      }
      const page = await res.json() as Page;
      setItems(page.items);
      setCursor(page.next_cursor);
      setConnected(true);
      setStatus(`Đã tải ${page.items.length} thành viên.`);
    } catch {
      setStatus('Lỗi mạng khi đọc memberships.');
    } finally {
      setBusy(false);
    }
  }

  async function createMembership() {
    const sub = subjectId.trim();
    const comp = companyId.trim();
    const roles = rolesText.split(',').map(s => s.trim()).filter(Boolean);
    if (!credential.current || !sub || !comp || roles.length === 0) return;
    setBusy(true);
    setStatus('Đang cấp quyền thành viên...');
    try {
      const res = await fetch('/v1/memberships', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': 'ui-mem-create-' + crypto.randomUUID(),
        },
        body: JSON.stringify({ subject_id: sub, company_id: comp, roles }),
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        setStatus(`Cấp quyền thất bại (${res.status}).`);
        return;
      }
      const created = await res.json() as Membership;
      setStatus(`Đã cấp quyền cho ${created.subject_id}.`);
      await load();
    } catch {
      setStatus('Lỗi kết nối khi cấp quyền.');
    } finally {
      setBusy(false);
    }
  }

  async function revokeMembership(targetSubject: string) {
    if (!credential.current || !revokeReason.trim()) return;
    setBusy(true);
    setStatus(`Đang lấy Finance-Version và thu hồi ${targetSubject}...`);
    try {
      const getRes = await fetch('/v1/memberships/' + encodeURIComponent(targetSubject), {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      const ver = getRes.headers.get('finance-version');
      if (!getRes.ok || !ver) {
        setStatus(`Không đọc được phiên bản thành viên (${getRes.status}).`);
        return;
      }
      const res = await fetch('/v1/memberships/' + encodeURIComponent(targetSubject) + '/revoke', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': 'ui-mem-revoke-' + crypto.randomUUID(),
          'If-Finance-Version': ver,
        },
        body: JSON.stringify({ reason: revokeReason.trim() }),
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        setStatus(`Thu hồi quyền thất bại (${res.status}).`);
        return;
      }
      setStatus(`Đã thu hồi quyền của ${targetSubject}.`);
      await load();
    } catch {
      setStatus('Lỗi kết nối khi thu hồi quyền.');
    } finally {
      setBusy(false);
    }
  }

  return <section aria-labelledby="e03-title">
    <h2 id="e03-title">Phân quyền &amp; Thành viên (E03)</h2>
    <div className="sandbox-connection">
      <label htmlFor="e03-token">Token OrgAdmin sandbox</label>
      <div className="actions">
        <input id="e03-token" type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} disabled={busy || connected} maxLength={16384} />
        <button className="primary" type="button" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); void load(); }}>Kết nối</button>
        <button type="button" onClick={disconnect}>Ngắt kết nối</button>
      </div>
    </div>
    <p role="status" aria-live="polite">{status}</p>
    <div className="actions">
      <input value={subjectId} onChange={e => setSubjectId(e.target.value)} placeholder="Subject UUID" aria-label="Subject UUID" />
      <input value={companyId} onChange={e => setCompanyId(e.target.value)} placeholder="Company UUID" aria-label="Company UUID" />
      <input value={rolesText} onChange={e => setRolesText(e.target.value)} placeholder="REQUESTER,AUDITOR" aria-label="Roles" />
      <button type="button" disabled={!connected || busy || !subjectId.trim()} onClick={() => void createMembership()}>Cấp quyền</button>
    </div>
    <div className="actions">
      <label htmlFor="e03-reason">Lý do thu hồi</label>
      <input id="e03-reason" value={revokeReason} onChange={e => setRevokeReason(e.target.value)} />
      <button type="button" disabled={!connected || busy} onClick={() => void load()}>Tải lại</button>
      <button type="button" disabled={!cursor || busy} onClick={() => void load(cursor)}>Trang tiếp</button>
    </div>
    <div className="table-wrap"><table><thead><tr><th>Subject</th><th>Company</th><th>Roles</th><th>Active</th><th>Thao tác</th></tr></thead>
      <tbody>{items.map(row => <tr key={row.id}>
        <td>{row.subject_id}</td><td>{row.company_id}</td><td>{row.roles.join(', ')}</td><td>{row.active ? 'Hoạt động' : 'Đã thu hồi'}</td>
        <td>{row.active && <button type="button" disabled={busy} onClick={() => void revokeMembership(row.subject_id)}>Thu hồi</button>}</td>
      </tr>)}</tbody>
    </table></div>
  </section>;
}
