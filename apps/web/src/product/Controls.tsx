import type {ButtonHTMLAttributes} from 'react';
type Props=ButtonHTMLAttributes<HTMLButtonElement>&{variant?:'primary'|'secondary'|'tab'|'icon'};
const variants={primary:'nv-btn',secondary:'nv-btn-secondary',tab:'nv-btn-tab',icon:'nv-btn-icon'} as const;
export function Button({variant='secondary',className='',type='button',...props}:Props){
 return <button type={type} className={'nv-control '+variants[variant]+(className?' '+className:'')} {...props}/>;
}
