import { ReferenceList } from '../financial/ReferenceList';
export function VerifiedBudgets({ token }: { token: string }) {
  return <ReferenceList title="Ngân sách đã xác minh" endpoint="/v1/budgets" token={token}/>;
}
