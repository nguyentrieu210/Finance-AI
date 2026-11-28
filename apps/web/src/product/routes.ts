import {contracts} from './contracts';
const titles=['Tổng quan','Hồ sơ của tôi','Tạo hồ sơ','Chi tiết hồ sơ','Chứng từ đính kèm','Xem chứng từ','Xác minh dữ liệu AI','Đối chiếu chứng từ','Checklist hồ sơ','Kiểm tra Preflight','Ngoại lệ','Xuất hồ sơ','Bàn giao hồ sơ','Hồ sơ chờ kiểm soát','Không gian kế toán','Trạng thái thanh toán','Chính sách','Soạn chính sách','Thiết kế quy tắc','Mô phỏng quy tắc','Thiết kế biểu mẫu','Mẫu tài liệu','Quy trình','Dữ liệu tham chiếu','Hạn mức & giữ chỗ','Tổ chức & phân quyền','Rủi ro & ngoại lệ','Đối soát tích hợp','Nhật ký kiểm toán','Giám sát vận hành'];
export const screens=contracts.map((r,i)=>({path:r[1],title:titles[i]??r[0]}));
export function resolve(path:string){return screens.find(s=>s.path===path)??screens.find(s=>s.path.includes(':id')&&s.path.split('/').length===path.split('/').length&&s.path.split('/').every((p,i)=>p===':id'?Boolean(path.split('/')[i]):p===path.split('/')[i]));}

export type Crumb={title:string;to?:string};
export function breadcrumbs(location:string):Crumb[]{
 const [path,search='']=location.split('?');
 const current=resolve(path);
 if(path==='/')return [{title:'Tổng quan'}];
 const out:Crumb[]=[{title:'Tổng quan',to:'/'}];
 const kind=new URLSearchParams(search).get('kind');
 const kindTitle=kind==='PAYMENT'?'Thanh toán':kind==='ADVANCE'?'Tạm ứng':kind==='SETTLEMENT'?'Quyết toán tạm ứng':undefined;
 if(path==='/dossiers'){
  if(kindTitle)out.push({title:'Hồ sơ của tôi',to:'/dossiers'},{title:kindTitle});
  else out.push({title:'Hồ sơ của tôi'});
  return out;
 }
 const parts=path.split('/');
 if(parts[1]==='dossiers'&&current){
  out.push({title:'Hồ sơ của tôi',to:'/dossiers'});
  if(parts[2]==='new'){if(kindTitle)out.push({title:kindTitle,to:'/dossiers?kind='+kind});out.push({title:'Tạo hồ sơ'});}
  else{if(parts.length>3)out.push({title:'Chi tiết hồ sơ',to:'/dossiers/'+parts[2]});out.push({title:current.title});}
  return out;
 }
 if(parts[1]==='review'&&parts.length>2&&current)out.push({title:'Hồ sơ chờ kiểm soát',to:'/review'});
 if(parts[1]==='admin'&&parts[2]==='policies'&&parts.length>3&&current)out.push({title:'Chính sách',to:'/admin/policies'});
 out.push({title:current?.title??'Không tìm thấy trang'});
 return out;
}
