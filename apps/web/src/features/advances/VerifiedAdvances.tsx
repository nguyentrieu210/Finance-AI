import { ReferenceList } from '../financial/ReferenceList';
export function VerifiedAdvances({ token }: { token: string }) {
  return <ReferenceList title="Tạm ứng đã xác minh" endpoint="/v1/advances" token={token}/>;
}
