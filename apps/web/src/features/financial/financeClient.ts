// Same-origin authenticated calls only; no persistent credentials.
export const isUuid=(s:string)=>/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)&&s!=='00000000-0000-0000-0000-000000000000';
export const unique=(s:string[])=>[...new Set(s)];
export const ids=(s:string)=>unique(s.split(/[\s,;]+/).filter(Boolean).map(x=>x.toLowerCase()));
export const isAmount=(s:string)=>/^\d{1,20}(\.\d{1,8})?$/.test(s);
export type ResourceKind='CONTRACT'|'INVOICE'|'ADVANCE'|'BUDGET';
export const kinds:ResourceKind[]=['CONTRACT','INVOICE','ADVANCE','BUDGET'];
export type Row={source_key:string;currency:string;cap:string;consumed:string;evidence_ids:string[]};
export type Material={id:string;document_id:string;source_digest:string;kind:ResourceKind;coverage:string;as_of:string;fresh_until:string;state:string;records:Row[];evidence_ids:string[];maker_id:string};
export type Resource={id:string;kind:ResourceKind;source_key:string;snapshot_id:string;currency:string;cap:string;consumed:string;held:string;available:string;fresh_until:string};
export type Gate={clause_key:string;amount_path:string;currency_path:string;resource_kinds:ResourceKind[]};
export type Config={id:string;company_id:string;version:number;name:string;state:string;effective_from:string;effective_to:string;author_id:string;criticality_complete:boolean;finance_gate:Gate|null;clauses:{key:string;id:string;excerpt:string;page:number}[];critical_fields:{clause_key:string;field_path:string}[];checklist:{clause_key:string;document_kind:string;mandatory:boolean}[]};
export async function api<T>(token:string,path:string,init:RequestInit={}):Promise<{data:T;version:string|null}>{
 if(!token||!path.startsWith('/v1/')||path.startsWith('//'))throw new Error('Phiên xác thực không hợp lệ');
 const headers=new Headers(init.headers);headers.set('Authorization','Bearer '+token);
 if(init.body)headers.set('Content-Type','application/json');
 const response=await fetch(path,{...init,headers,credentials:'omit',cache:'no-store',redirect:'error'});
 if(!response.ok)throw new Error('HTTP '+response.status);
 return {data:await response.json() as T,version:response.headers.get('finance-version')};
}
export function key(prefix:string){return prefix+'-'+crypto.randomUUID()}
export function errorMessage(e:unknown){return e instanceof Error?e.message:'Lỗi kết nối'}
