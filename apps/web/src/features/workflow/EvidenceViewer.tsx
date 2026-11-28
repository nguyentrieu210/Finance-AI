import { useEffect, useState } from 'react';
import type { operations } from '../../generated/api';

type Evidence = operations['getEvidence']['responses'][200]['content']['application/json'];

export function EvidenceViewer({ id, token, onClose }: { id: string; token: string; onClose: () => void }) {
  const [value, setValue] = useState<Evidence>();
  const [message, setMessage] = useState('Đang đọc bằng chứng...');
  useEffect(() => {
    const controller = new AbortController();
    setValue(undefined);
    setMessage('Đang đọc bằng chứng...');
    void (async () => {
      try {
        const response = await fetch('/v1/evidence/' + encodeURIComponent(id), {
          headers: { Authorization: 'Bearer ' + token },
          credentials: 'omit', cache: 'no-store', signal: controller.signal,
        });
        if (!response.ok) throw new Error(String(response.status));
        const evidence = await response.json() as Evidence;
        if (evidence.id !== id || typeof evidence.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(evidence.sha256)) {
          throw new Error('invalid evidence');
        }
        if (controller.signal.aborted) return;
        setValue(evidence);
        setMessage('Bằng chứng trích xuất; cần đối chiếu với tài liệu gốc.');
      } catch {
        if (!controller.signal.aborted) setMessage('Không tìm thấy bằng chứng hoặc không có quyền truy cập.');
      }
    })();
    return () => controller.abort();
  }, [id, token]);

  async function downloadRendered() {
    if (!value || !value.locator.startsWith('derived-pdf:')) return;
    try {
      const response = await fetch('/v1/evidence/' + encodeURIComponent(id) + '/rendered', {
        headers: { Authorization: 'Bearer ' + token }, credentials: 'omit', cache: 'no-store',
      });
      if (!response.ok) throw new Error('download');
      const bytes = await response.arrayBuffer();
      if (bytes.byteLength > 25 * 1024 * 1024) throw new Error('size');
      const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)), b=>b.toString(16).padStart(2,'0')).join('');
      if (digest !== value.locator.split(':')[1] || digest !== response.headers.get('x-content-sha256')) throw new Error('digest');
      const url = URL.createObjectURL(new Blob([bytes], { type: 'application/pdf' }));
      const link = document.createElement('a'); link.href = url; link.download='rendered-evidence.pdf'; link.click();
      window.setTimeout(()=>URL.revokeObjectURL(url),1000);
    } catch { setMessage('Không tải được bản PDF đối chiếu hoặc SHA-256 không khớp.'); }
  }

  return <section aria-label="Chi tiết bằng chứng">
    <h4>Chi tiết bằng chứng</h4>
    <button type="button" onClick={onClose}>Đóng bằng chứng</button>
    <p role="status">{message}</p>
    {value && <dl>
      <dt>Tài liệu nguồn</dt><dd>{value.document_id}</dd>
      <dt>Trang</dt><dd>{value.page ?? 'Không xác định'}</dd>
      <dt>Vị trí</dt><dd>{value.locator}</dd>
      {value.locator.startsWith('derived-pdf:') && <><dt>Bản đối chiếu</dt><dd><button type="button" onClick={()=>void downloadRendered()}>Tải PDF kết xuất</button> (không phải tài liệu Office gốc)</dd></>}
      <dt>Nội dung OCR</dt><dd>{value.excerpt ?? 'Không có nội dung'}</dd>
      <dt>SHA-256</dt><dd style={{ overflowWrap: 'anywhere' }}>{value.sha256}</dd>
    </dl>}
  </section>;
}
