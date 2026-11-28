<#import "template.ftl" as layout>
<@layout.registrationLayout displayMessage=false; section>
 <#if section = "header">
  Không thể tiếp tục đăng nhập
 <#elseif section = "form">
  <div id="kc-error-message">
   <p class="instruction">Phiên đăng nhập không còn hợp lệ hoặc yêu cầu xác thực gặp lỗi.</p>
   <p class="instruction">Vui lòng mở một phiên đăng nhập mới.</p>
   <a class="nv-auth-restart" href="https://namviet.gpcds.site/">Đăng nhập lại</a>
   <#if traceId??><p class="instruction" id="traceId">${msg("traceIdSupportMessage", traceId)}</p></#if>
  </div>
 </#if>
</@layout.registrationLayout>
