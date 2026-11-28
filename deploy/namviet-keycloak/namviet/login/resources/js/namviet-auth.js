(function(){
 function init(){
  var root=document.documentElement,theme;
  try{theme=localStorage.getItem('nv.theme')}catch(e){}
  if(theme!=='dark'&&theme!=='light')theme=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';
  var button=document.createElement('button');button.type='button';button.className='nv-auth-theme-toggle';
  function apply(){
   root.dataset.nvTheme=theme;
   button.textContent=theme==='dark'?'☀':'☾';
   button.setAttribute('aria-label',theme==='dark'?'Chuyển nền sáng':'Chuyển nền tối');
   button.title=button.getAttribute('aria-label');
   try{localStorage.setItem('nv.theme',theme)}catch(e){}
  }
  button.addEventListener('click',function(){theme=theme==='dark'?'light':'dark';apply()});
  apply();document.body.appendChild(button);
 }
 if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',init);else init();
})();
