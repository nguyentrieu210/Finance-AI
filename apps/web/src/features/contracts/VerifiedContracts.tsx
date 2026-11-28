import { ReferenceList } from '../financial/ReferenceList';
export function VerifiedContracts({ token }: { token: string }) {
  return <ReferenceList title="Hợp đồng đã xác minh" endpoint="/v1/contracts" token={token}/>;
}
