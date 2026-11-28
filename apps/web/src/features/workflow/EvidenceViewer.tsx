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

  return <section aria-label="Chi tiết bằng chứng">
    <h4>Chi tiết bằng chứng</h4>
    <button type="button" onClick={onClose}>Đóng bằng chứng</button>
    <p role="status">{message}</p>
    {value && <dl>
      <dt>Tài liệu nguồn</dt><dd>{value.document_id}</dd>
      <dt>Trang</dt><dd>{value.page ?? 'Không xác định'}</dd>
      <dt>Vị trí</dt><dd>{value.locator}</dd>
      <dt>Nội dung OCR</dt><dd>{value.excerpt ?? 'Không có nội dung'}</dd>
      <dt>SHA-256</dt><dd style={{ overflowWrap: 'anywhere' }}>{value.sha256}</dd>
    </dl>}
  </section>;
}
