<#import "template.ftl" as layout>
<@layout.registrationLayout displayMessage=false; section>
 <#if section = "header">
  Phiên đăng nhập đã hết hạn
 <#elseif section = "form">
  <p class="instruction">Vui lòng bắt đầu lại từ cổng đăng nhập Nam Việt.</p>
  <a class="nv-auth-restart" href="https://namviet.gpcds.site/">Đăng nhập lại</a>
 </#if>
</@layout.registrationLayout>
