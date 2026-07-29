import{o as e,t}from"./chunk-CFjPhJqf.js";import{n,r,t as i}from"./jsx-runtime-Bbdm_1Ch.js";import{Br as a,Cr as o,Fr as s,Ir as c,N as l,Pr as u,Rr as d,Sr as f,_t as p,br as m,ct as h,n as g,st as _}from"./spinner-Do7-KqTg.js";import{F as v,J as y,M as b,O as x,St as S,T as C,X as w,an as T,gt as E,ht as D,k as O,kt as k,mt as A,nt as j,q as M,tt as N,vt as P,wt as F}from"./button-NT8jlGNO.js";import{Dl as I,Su as L,at as ee,mu as R}from"./app-server-manager-signals-B7LUkeum.js";import{i as te,n as ne}from"./react-DdkVu0Rl.js";import{t as re}from"./queryOptions-BB5wRbO-.js";import{s as ie}from"./x-CkKiYZJy.js";import{A as ae,m as oe}from"./config-queries-BeRz7q3T.js";import{n as z,t as B}from"./browser-sidebar-availability-DYLvJQQ8.js";import{t as V}from"./use-platform-CSln2-Nu.js";import{i as se,n as H,s as U}from"./use-debounced-value-Sm8bVZ4R.js";import{t as W}from"./uniq-Cu5Al9TL.js";import{t as G}from"./sites-color-DY2Txhg1.js";import{t as ce}from"./_baseSlice-BCbe-1RS.js";import{r as le}from"./skus-wlDWgxO_.js";import{n as ue,t as de}from"./branch-CcsKz667.js";import{n as fe,t as pe}from"./use-is-dark-DjElvZcp.js";import{i as K,r as q}from"./use-skills-B4PTpIYV.js";var J=n();function Y(e){let t=(0,J.c)(10),{hostId:n,featureName:r,defaultEnabled:i}=e,a=i===void 0?!0:i,{data:o,isLoading:s}=S(z,n),c;t[0]===o?c=t[1]:(c=o===void 0?[]:o,t[0]=o,t[1]=c);let l=c,u;if(t[2]!==r||t[3]!==l){let e;t[5]===r?e=t[6]:(e=e=>e.name===r,t[5]=r,t[6]=e),u=l.find(e),t[2]=r,t[3]=l,t[4]=u}else u=t[4];let d=u?.enabled??a,f;return t[7]!==s||t[8]!==d?(f={enabled:d,isLoading:s},t[7]=s,t[8]=d,t[9]=f):f=t[9],f}var me=e(r(),1);function he(e){return e===`macOS`||e===`windows`}function ge(e){let t=(0,J.c)(16),{enabled:n,hostId:r}=e,i=n===void 0?!0:n,{isLoading:a,platform:o}=V(),s=ie(`1506311413`),c;t[0]===r?c=t[1]:(c={featureName:`computer_use`,hostId:r},t[0]=r,t[1]=c);let l=Y(c),u=o===`windows`&&!a,d=i&&u,f;t[2]===d?f=t[3]:(f={enabled:d},t[2]=d,t[3]=f);let p=_e(f),m=l.isLoading||u&&p.isLoading,h=l.enabled&&(!u||p.enabled),g;t[4]!==h||t[5]!==i||t[6]!==m||t[7]!==s||t[8]!==a||t[9]!==o?(g=ye({areRequiredFeaturesEnabled:h,enabled:i,isAnyFeatureLoading:m,isComputerUseGateEnabled:s,isHostCompatiblePlatform:he(o),isPlatformLoading:a,windowType:`chrome-extension`}),t[4]=h,t[5]=i,t[6]=m,t[7]=s,t[8]=a,t[9]=o,t[10]=g):g=t[10];let _=g,v=_===`available`,y=_===`loading`&&m,b=_===`loading`,x;return t[11]!==_||t[12]!==v||t[13]!==y||t[14]!==b?(x={available:v,isFetching:y,isLoading:b,reason:_},t[11]=_,t[12]=v,t[13]=y,t[14]=b,t[15]=x):x=t[15],x}function _e(e){let t=(0,J.c)(21),{enabled:n}=e,r=(0,me.useContext)(U)?.authMethod===`chatgpt`,i;t[0]===Symbol.for(`react.memo_cache_sentinel`)?(i=[`accounts`,`check`],t[0]=i):i=t[0];let a=n&&r,o;t[1]===a?o=t[2]:(o={queryKey:i,queryFn:ve,staleTime:D.ONE_MINUTE,enabled:a},t[1]=a,t[2]=o);let{data:s,errorUpdatedAt:c,isLoading:l}=j(o),u=s?.account_ordering?.[0],d;t[3]!==s?.accounts||t[4]!==u?(d=s?.accounts?.find(e=>e.id===u),t[3]=s?.accounts,t[4]=u,t[5]=d):d=t[5];let f=d,p=f==null&&(!l||c!==0),m=n&&r&&p,h;t[6]===m?h=t[7]:(h={queryConfig:{enabled:m,staleTime:D.ONE_MINUTE}},t[6]=m,t[7]=h);let{data:g,isLoading:_}=b(`account-info`,h),v=f?.id??(p?g?.accountId:void 0),x=f?.plan_type??(p?g?.plan:void 0),S=r?x:void 0,C;t[8]===S?C=t[9]:(C=le(S),t[8]=S,t[9]=C);let w=C,T;t[10]===v?T=t[11]:(T=[`accounts`,`settings`,v],t[10]=v,t[11]=T);let E=n&&v!=null&&w&&r,O;t[12]===v?O=t[13]:(O=async()=>y.safeGet(`/accounts/{account_id}/settings`,{parameters:{path:{account_id:v??``}}}),t[12]=v,t[13]=O);let k;t[14]!==E||t[15]!==O||t[16]!==T?(k={queryKey:T,enabled:E,queryFn:O,staleTime:D.ONE_MINUTE},t[14]=E,t[15]=O,t[16]=T,t[17]=k):k=t[17];let{data:A,isLoading:M}=j(k),N=!r||x!=null&&(!w||(A?.beta_settings?.windows_computer_use??!1)),P=n&&r&&(l&&!p||_||M),F;return t[18]!==N||t[19]!==P?(F={enabled:N,isLoading:P},t[18]=N,t[19]=P,t[20]=F):F=t[20],F}async function ve(){return y.safeGet(`/wham/accounts/check`)}function ye({areRequiredFeaturesEnabled:e,enabled:t,isAnyFeatureLoading:n,isComputerUseGateEnabled:r,isHostCompatiblePlatform:i,isPlatformLoading:a,windowType:o}){return t?o===`electron`?r?a?`loading`:i?n?`loading`:e?`available`:`config-requirement-disabled`:`unsupported-platform`:`statsig-disabled`:`window-type-disabled`:`disabled`}function be(e){let t=(0,J.c)(12),{hostId:n,windowType:r}=e,i=r===void 0?`chrome-extension`:r,a=ie(`410065390`),o;t[0]===n?o=t[1]:(o={featureName:`browser_use_external`,hostId:n},t[0]=n,t[1]=o);let s=Y(o),c;t[2]!==s.enabled||t[3]!==s.isLoading||t[4]!==a||t[5]!==i?(c=xe({isExternalBrowserUseFeatureEnabled:s.enabled,isExternalBrowserUseFeatureLoading:s.isLoading,isExternalBrowserUseGateEnabled:a,windowType:i}),t[2]=s.enabled,t[3]=s.isLoading,t[4]=a,t[5]=i,t[6]=c):c=t[6];let l=c,u=l===`available`,d=l===`available`,f=l===`loading`,p;return t[7]!==l||t[8]!==u||t[9]!==d||t[10]!==f?(p={allowed:u,available:d,isLoading:f,reason:l},t[7]=l,t[8]=u,t[9]=d,t[10]=f,t[11]=p):p=t[11],p}function xe({isExternalBrowserUseFeatureEnabled:e,isExternalBrowserUseFeatureLoading:t,isExternalBrowserUseGateEnabled:n,windowType:r}){return r===`chrome-extension`?`available`:t?`loading`:n?e?`available`:`config-requirement-disabled`:`statsig-disabled`}function Se(e){let t=(0,J.c)(13),{hostId:n}=e,r=F(B),i=ie(`410262010`),a;t[0]===n?a=t[1]:(a={featureName:`browser_use`,hostId:n},t[0]=n,t[1]=a);let o=Y(a),s=C(l.runCodexInWsl),c=o.enabled&&!o.isLoading,u=o.isLoading,d=s===!0,f;t[2]!==i||t[3]!==r||t[4]!==c||t[5]!==u||t[6]!==d?(f=Ce({isBrowserAgentGateEnabled:i,isBrowserSidebarEnabled:r,isBrowserUseEnabled:c,isLoading:u,runCodexInWsl:d,windowType:`chrome-extension`}),t[2]=i,t[3]=r,t[4]=c,t[5]=u,t[6]=d,t[7]=f):f=t[7];let p=f,m=p===`available`,h=p===`available`,g=p===`loading`,_;return t[8]!==p||t[9]!==m||t[10]!==h||t[11]!==g?(_={allowed:m,available:h,isLoading:g,reason:p},t[8]=p,t[9]=m,t[10]=h,t[11]=g,t[12]=_):_=t[12],_}function Ce({isBrowserAgentGateEnabled:e,isBrowserSidebarEnabled:t,isBrowserUseEnabled:n,isLoading:r,runCodexInWsl:i,windowType:a}){return a===`chrome-extension`?`window-type-disabled`:r?`loading`:t?e?n?i?`wsl-disabled`:`available`:`config-requirement-disabled`:`statsig-disabled`:`browser-pane-disabled`}var we=`plugins`;function Te(e){let t=(0,J.c)(4),{hostId:n}=e,{data:r}=S(z,n),i;t[0]===r?i=t[1]:(i=r===void 0?[]:r,t[0]=r,t[1]=i);let a=i,o;return t[2]===a?o=t[3]:(o=a.find(Ee),t[2]=a,t[3]=o),o?.enabled??!0}function Ee(e){return e.name===we}var De=`<svg width="192" height="192" viewBox="0 0 192 192" fill="none" xmlns="http://www.w3.org/2000/svg">
<mask id="google-docs-2026-mask0_37242_8762" style="mask-type:alpha" maskUnits="userSpaceOnUse" x="32" y="8" width="128" height="176">
<path d="M130.334 184L61.6 184C52.6565 184 48.1848 184 44.6375 182.596C39.5029 180.563 35.4374 176.497 33.4045 171.362C32 167.815 32 163.343 32 154.4L32 37.6C32 28.6565 32 24.1848 33.4045 20.6375C35.4374 15.5029 39.5029 11.4374 44.6375 9.40447C48.1848 8 52.6565 8 61.6 8L100 8L154.793 62.7933L154.793 62.7934C156.454 64.4543 157.285 65.2848 157.923 66.2239C158.845 67.5811 159.479 69.1131 159.785 70.725C159.997 71.8404 159.997 73.0264 159.995 75.3985C159.96 124.317 159.938 124.799 159.937 154.366C159.937 163.332 159.937 167.816 158.532 171.363C156.499 176.498 152.434 180.562 147.299 182.596C143.752 184 139.279 184 130.334 184Z" fill="#3186FF"/>
</mask>
<g mask="url(#google-docs-2026-mask0_37242_8762)">
<path d="M159.94 184L31.9999 184L31.9999 8.00001L99.9999 8L159.999 68L159.94 184Z" fill="#3186FF"/>
<g filter="url(#google-docs-2026-filter0_f_37242_8762)">
<path d="M43 192H149V70.2271V20H43V192Z" fill="url(#google-docs-2026-paint0_linear_37242_8762)"/>
</g>
</g>
<path d="M154.995 62.9951C152.489 61.1143 149.375 60 146 60H112.8C105.731 60 100 54.2692 100 47.2002V8L154.995 62.9951Z" fill="#76BBFF"/>
<rect x="64.001" y="114" width="64" height="12" rx="6" fill="white"/>
<rect x="64.001" y="143" width="48" height="12" rx="6" fill="white"/>
<defs>
<filter id="google-docs-2026-filter0_f_37242_8762" x="31" y="8" width="130" height="196" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
<feFlood flood-opacity="0" result="BackgroundImageFix"/>
<feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
<feGaussianBlur stdDeviation="6" result="effect1_foregroundBlur_37242_8762"/>
</filter>
<linearGradient id="google-docs-2026-paint0_linear_37242_8762" x1="96" y1="59.2839" x2="54.6124" y2="171.338" gradientUnits="userSpaceOnUse">
<stop offset="0.33" stop-color="#3186FF"/>
<stop offset="1" stop-color="#A9A8FF"/>
</linearGradient>
</defs>
</svg>
`,Oe=`<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192" fill="none" viewBox="0 0 192 192"><path fill="#009954" d="M8 74.6c0-8.943 0-13.415 1.404-16.962a20 20 0 0 1 11.234-11.233C24.185 45 28.656 45 37.6 45h60.8c8.943 0 13.415 0 16.962 1.404a20 20 0 0 1 11.234 11.234C128 61.185 128 65.656 128 74.6v42.8c0 8.943 0 13.415-1.404 16.962a20 20 0 0 1-11.234 11.234C111.815 147 107.343 147 98.4 147H37.6c-8.943 0-13.415 0-16.963-1.404a20 20 0 0 1-11.233-11.234C8 130.815 8 126.343 8 117.4z"/><mask id="google-sheets-2026-a" width="160" height="128" x="24" y="32" maskUnits="userSpaceOnUse" style="mask-type:alpha"><rect width="160" height="128" x="24" y="32" fill="#0ebc5f" rx="20"/></mask><g mask="url(#google-sheets-2026-a)"><path fill="#0ebc5f" d="M24 32h160v128H24z"/><g filter="url(#google-sheets-2026-b)"><rect width="144" height="102" fill="url(#google-sheets-2026-c)" rx="25.6" transform="matrix(1 0 0 -1 8 147)"/></g></g><path stroke="#fff" stroke-linecap="round" stroke-width="12" d="M80 121h84m-20 19V76"/><defs><linearGradient id="google-sheets-2026-c" x1="122.24" x2="20.76" y1="43.31" y2="43.31" gradientUnits="userSpaceOnUse"><stop stop-color="#0ebc5f"/><stop offset=".95" stop-color="#78c9ff"/></linearGradient><filter id="google-sheets-2026-b" width="168" height="126" x="-4" y="33" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse"><feFlood flood-opacity="0" result="BackgroundImageFix"/><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"/><feGaussianBlur result="effect1_foregroundBlur_37435_8174" stdDeviation="6"/></filter></defs></svg>`,ke=`<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192" fill="none" viewBox="0 0 192 192"><path fill="url(#google-slides-2026-a)" d="M12.591 63.318c-2.493-15.262 7.858-29.655 23.12-32.148l96.724-15.8c15.262-2.492 29.655 7.859 32.148 23.12l14.732 90.189c2.493 15.262-7.858 29.655-23.12 32.148l-96.724 15.8c-15.262 2.493-29.655-7.858-32.148-23.12z"/><path fill="url(#google-slides-2026-b)" d="M12 61.6c0-8.943 0-13.415 1.405-16.962a20 20 0 0 1 11.233-11.233C28.185 32 32.656 32 41.6 32h108.8c8.943 0 13.415 0 16.962 1.404a20 20 0 0 1 11.234 11.234C180 48.185 180 52.657 180 61.6v68.8c0 8.943 0 13.415-1.404 16.962a20 20 0 0 1-11.234 11.234C163.815 160 159.343 160 150.4 160H41.6c-8.943 0-13.415 0-16.963-1.404a20 20 0 0 1-11.232-11.234C12 143.815 12 139.343 12 130.4z"/><mask id="google-slides-2026-e" width="168" height="128" x="12" y="32" maskUnits="userSpaceOnUse" style="mask-type:alpha"><path fill="#fec700" d="M12 61.6c0-8.943 0-13.415 1.405-16.962a20 20 0 0 1 11.233-11.233C28.185 32 32.656 32 41.6 32h108.8c8.943 0 13.415 0 16.962 1.404a20 20 0 0 1 11.234 11.234C180 48.185 180 52.657 180 61.6v68.8c0 8.943 0 13.415-1.404 16.962a20 20 0 0 1-11.234 11.234C163.815 160 159.343 160 150.4 160H41.6c-8.943 0-13.415 0-16.963-1.404a20 20 0 0 1-11.232-11.234C12 143.815 12 139.343 12 130.4z"/></mask><g filter="url(#google-slides-2026-c)" mask="url(#google-slides-2026-e)"><path fill="#ffbe00" d="m33.74 191.516 144.396-21.58L153.304 3.781 8.907 25.361z"/><path fill="url(#google-slides-2026-f)" d="m33.74 191.516 144.396-21.58L153.304 3.781 8.907 25.361z"/></g><path fill="#fff" fill-rule="evenodd" d="M148 58a6 6 0 0 1 6 6v64a6 6 0 0 1-6 6H44l-.309-.008A6 6 0 0 1 38 128V64a6 6 0 0 1 5.691-5.992L44 58zm-98 64h92V70H50z" clip-rule="evenodd"/><defs><linearGradient id="google-slides-2026-a" x1="84.07" x2="157.2" y1="23.27" y2="160.82" gradientUnits="userSpaceOnUse"><stop offset=".2" stop-color="#ffdb0f"/><stop offset=".67" stop-color="#ffbe00"/><stop offset=".91" stop-color="#ffa8e3"/></linearGradient><linearGradient id="google-slides-2026-b" x1="96" x2="96" y1="32" y2="160" gradientUnits="userSpaceOnUse"><stop stop-color="#ffbe00"/><stop offset="1" stop-color="#fec700"/></linearGradient><linearGradient id="google-slides-2026-f" x1="108.52" x2="83.96" y1="168.16" y2="25.27" gradientUnits="userSpaceOnUse"><stop offset=".07" stop-color="#fff549"/><stop offset=".78" stop-color="#ffbe00" stop-opacity="0"/></linearGradient><filter id="google-slides-2026-c" width="193.23" height="211.73" x="-3.09" y="-8.22" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse"><feFlood flood-opacity="0" result="BackgroundImageFix"/><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"/><feGaussianBlur result="effect1_foregroundBlur_37552_9023" stdDeviation="6"/></filter></defs></svg>`,Ae=`<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192" fill="none" viewBox="0 0 192 192"><path fill="url(#gmail-2026-a)" d="M146 44h38v110c0 6.627-5.373 12-12 12h-20a6 6 0 0 1-6-6z"/><path fill="#fc413d" d="M46 44H8v110c0 6.627 5.373 12 12 12h20a6 6 0 0 0 6-6z"/><path fill="url(#gmail-2026-b)" d="M39.226 30.456c-8.033-6.752-20.018-5.714-26.77 2.319-6.752 8.032-5.714 20.017 2.319 26.77l76.078 63.949a8 8 0 0 0 10.295 0l76.078-63.95c8.032-6.752 9.07-18.737 2.318-26.77-6.752-8.032-18.737-9.07-26.769-2.318L96 78.18z"/><defs><linearGradient id="gmail-2026-a" x1="165" x2="165" y1="44" y2="166" gradientUnits="userSpaceOnUse"><stop stop-color="#60d673"/><stop offset=".17" stop-color="#42c868"/><stop offset=".39" stop-color="#0ebc5f"/><stop offset=".62" stop-color="#00a9bb"/><stop offset=".86" stop-color="#3c90ff"/><stop offset="1" stop-color="#3186ff"/></linearGradient><linearGradient id="gmail-2026-b" x1="8" x2="184" y1="46.13" y2="46.13" gradientUnits="userSpaceOnUse"><stop offset=".08" stop-color="#ff63a0"/><stop offset=".3" stop-color="#fc413d"/><stop offset=".5" stop-color="#fc413d"/><stop offset=".65" stop-color="#fc413d"/><stop offset=".72" stop-color="#fc5c30"/><stop offset=".86" stop-color="#feb10c"/><stop offset=".91" stop-color="#fec700"/><stop offset=".96" stop-color="#ffdb0f"/></linearGradient></defs></svg>`,je=`<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192" viewBox="0 0 192 192" fill="none"><path fill="#bbe2ff" d="M32 36.8C32 20.894 44.894 8 60.8 8h70.4C147.106 8 160 20.894 160 36.8v30.4c0 15.906-12.894 28.8-28.8 28.8H60.8C44.894 96 32 83.106 32 67.2z"/><path fill="#3c90ff" d="M19.867 49.392C17.818 33.82 29.94 20 45.645 20h100.71c15.706 0 27.827 13.82 25.778 29.392L166 96l6.133 46.608C174.182 158.18 162.061 172 146.355 172H45.645c-15.706 0-27.827-13.82-25.778-29.392L26 96z"/><mask id="google-calendar-2026-a" width="154" height="152" x="19" y="20" maskUnits="userSpaceOnUse" style="mask-type:alpha"><path fill="#3c90ff" d="M19.867 49.392C17.818 33.82 29.94 20 45.645 20h100.71c15.706 0 27.827 13.82 25.778 29.392L166 96l6.133 46.608C174.182 158.18 162.061 172 146.355 172H45.645c-15.706 0-27.827-13.82-25.778-29.392L26 96z"/></mask><g mask="url(#google-calendar-2026-a)"><path fill="url(#google-calendar-2026-b)" d="M0 0h166v76H0z" transform="matrix(1 0 0 -1 13 172)"/></g><mask id="google-calendar-2026-c" width="154" height="152" x="19" y="20" maskUnits="userSpaceOnUse" style="mask-type:alpha"><path fill="#3186ff" d="M19.867 49.392C17.818 33.82 29.94 20 45.645 20h100.71c15.706 0 27.827 13.82 25.778 29.392L166 96l6.133 46.608C174.182 158.18 162.061 172 146.355 172H45.645c-15.706 0-27.827-13.82-25.778-29.392L26 96z"/></mask><g mask="url(#google-calendar-2026-c)"><path fill="url(#google-calendar-2026-d)" d="M32 27.2C32 16.596 40.596 8 51.2 8h89.6c10.604 0 19.2 8.596 19.2 19.2V96H32z" filter="url(#google-calendar-2026-e)"/></g><path fill="#fff" d="M75.353 133.336q-6.282 0-10.777-2.043t-7.61-5.465q-3.065-3.474-4.342-6.793T51.603 115a2.07 2.07 0 0 1 1.021-1.124l5.67-2.247q.714-.357 1.43-.102.714.204 1.685 2.349 1.022 2.145 2.86 4.546a14.3 14.3 0 0 0 4.495 3.728q2.606 1.328 6.435 1.328 6.18 0 9.807-3.575 3.677-3.575 3.677-9.091 0-5.976-3.882-9.194-3.881-3.269-10.266-3.269h-5.362a1.9 1.9 0 0 1-1.328-.51q-.51-.562-.511-1.277v-5.465q0-.767.51-1.277a1.82 1.82 0 0 1 1.329-.562h4.647q5.721 0 9.194-3.116t3.473-8.07q0-4.902-3.116-7.916t-8.58-3.014q-3.065 0-5.312 1.022a11.5 11.5 0 0 0-3.882 2.86 22.7 22.7 0 0 0-2.809 3.78q-1.174 1.941-1.89 2.145-.714.153-1.379-.255l-5.363-2.605q-.664-.358-.868-1.124t1.226-3.575q1.481-2.86 4.494-5.823a21 21 0 0 1 7.049-4.597q4.035-1.635 9.398-1.634 9.96 0 15.782 5.26 5.823 5.21 5.823 13.791 0 5.925-2.86 10.266-2.81 4.34-7.968 6.13v.204q6.231 1.838 9.806 6.741 3.627 4.853 3.626 11.594 0 9.654-6.742 15.834-6.74 6.18-17.57 6.18zm51.25-1.175q-.868 0-1.533-.664a2.25 2.25 0 0 1-.612-1.583V73.118l-11.492 8.274q-.614.46-1.431.307a1.96 1.96 0 0 1-1.225-.766l-3.32-4.7a1.98 1.98 0 0 1-.358-1.43q.153-.816.817-1.276l20.379-14.557q.256-.204.562-.306.307-.153.715-.153h4.291q.868 0 1.379.613.562.56.562 1.43v69.36q0 .92-.664 1.583a2 2 0 0 1-1.533.664z"/><defs><linearGradient id="google-calendar-2026-b" x1="83" x2="83" y1="76" gradientUnits="userSpaceOnUse"><stop stop-color="#4fa0ff"/><stop offset="1" stop-color="#3186ff"/></linearGradient><linearGradient id="google-calendar-2026-d" x1="89.06" x2="89.06" y1="21.75" y2="96.39" gradientUnits="userSpaceOnUse"><stop stop-color="#a9a8ff"/><stop offset=".8" stop-color="#3c90ff"/></linearGradient><filter id="google-calendar-2026-e" width="152" height="112" x="20" y="-4" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse"><feFlood flood-opacity="0" result="BackgroundImageFix"/><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"/><feGaussianBlur result="effect1_foregroundBlur_37330_7673" stdDeviation="6"/></filter></defs></svg>`,Me=`<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192" viewBox="0 0 192 192" fill="none"><mask id="google-drive-2026-a" width="168" height="154" x="12" y="18" maskUnits="userSpaceOnUse" style="mask-type:alpha"><path fill="#b43333" d="M63.09 37c14.626-25.333 51.193-25.334 65.819 0l45.033 78c14.626 25.334-3.657 57.001-32.91 57.001H50.967c-29.253 0-47.536-31.667-32.91-57.001z"/></mask><g mask="url(#google-drive-2026-a)"><path fill="url(#google-drive-2026-b)" d="M206.905 172.02h-91.888l-19.015-32.934 45.944-79.578z"/><path fill="url(#google-drive-2026-c)" d="M-14.919 172.006 50.04 59.494v.002L31.032 92.422h38.02L115 172.004l-129.918.001z"/><path fill="url(#google-drive-2026-d)" d="M96.007-20.085 141.954 59.5l-19.011 32.928H31.048z"/></g><defs><linearGradient id="google-drive-2026-b" x1="193.6" x2="103.09" y1="165.6" y2="111.21" gradientUnits="userSpaceOnUse"><stop offset=".09" stop-color="#ffe921"/><stop offset="1" stop-color="#fec700"/></linearGradient><linearGradient id="google-drive-2026-c" x1="114.4" x2="15.53" y1="181.61" y2="121.8" gradientUnits="userSpaceOnUse"><stop offset=".15" stop-color="#a9a8ff"/><stop offset=".33" stop-color="#6d97ff"/><stop offset=".48" stop-color="#3186ff"/></linearGradient><linearGradient id="google-drive-2026-d" x1="128.88" x2="28.7" y1="37.88" y2="84.64" gradientUnits="userSpaceOnUse"><stop offset=".55" stop-color="#0ebc5f"/><stop offset=".85" stop-color="#78c9ff"/></linearGradient></defs></svg>`,Ne={gmail:X(Ae),"google-calendar":X(je),"google-docs":X(De),"google-drive":X(Me),"google-sheets":X(Oe),"google-slides":X(ke)};function Pe(e){let t=[e.name,e.id,e.interface?.displayName??``].map(Fe);for(let e of t){let t=Ie(e);if(t!=null)return t}return null}function X(e){let t=e.trim().replace(/^<svg\b[^>]*>/u,`<svg x="24" y="24" width="144" height="144" viewBox="0 0 192 192">`);return`data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="192" height="192" viewBox="0 0 192 192"><rect width="192" height="192" rx="40" fill="#fff"/>${t}</svg>`)}`}function Fe(e){return(e.split(`@`)[0]??``).trim().toLowerCase().split(/[^a-z0-9]+/g).filter(e=>e.length>0).join(`-`)}function Ie(e){let t=e.replace(/^connector-/u,``);return Ne[e]??Ne[t]??Ne[t.replace(/-mcp-server$/u,``)]??null}function Le(e){return e!==`chatgpt`&&e!==`apikey`&&e!==`amazonBedrock`}[`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#EBEBEB"
    d="M6 21c-.775 0-1.467-.167-2.075-.5A3.66 3.66 0 0 1 2.5 19.075c-.333-.608-.5-1.3-.5-2.075V7c0-.775.167-1.467.5-2.075a3.575 3.575 0 0 1 1.425-1.412C4.533 3.17 5.225 3 6 3h12c.775 0 1.467.17 2.075.513.608.333 1.08.804 1.413 1.412.341.608.512 1.3.512 2.075v10c0 .775-.17 1.467-.512 2.075a3.576 3.576 0 0 1-1.413 1.425c-.608.333-1.3.5-2.075.5H6Z"
  />
  <path
    fill="#2E9EFF"
    d="M18 3c.775 0 1.467.171 2.075.513a3.492 3.492 0 0 1 1.412 1.412C21.83 5.533 22 6.225 22 7v2H2V7c0-.775.167-1.467.5-2.075a3.575 3.575 0 0 1 1.425-1.412C4.533 3.17 5.225 3 6 3h12Z"
  />
  <path
    fill="#77C0FF"
    d="M8.287 6.713c.2.191.438.287.713.287a.953.953 0 0 0 .7-.287c.2-.2.3-.438.3-.713 0-.275-.1-.508-.3-.7A.933.933 0 0 0 9 5c-.275 0-.512.1-.713.3A.953.953 0 0 0 8 6c0 .275.096.513.287.713Zm3 0c.2.191.438.287.713.287a.953.953 0 0 0 .7-.287c.2-.2.3-.438.3-.713 0-.275-.1-.508-.3-.7A.933.933 0 0 0 12 5c-.275 0-.512.1-.713.3A.953.953 0 0 0 11 6c0 .275.096.513.287.713Zm-6 0c.2.191.438.287.713.287a.953.953 0 0 0 .7-.287c.2-.2.3-.438.3-.713 0-.275-.1-.508-.3-.7A.933.933 0 0 0 6 5c-.275 0-.513.1-.713.3A.953.953 0 0 0 5 6c0 .275.096.513.287.713Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#FFD400"
    d="M10.56 11.133v11.939h-.035a2.318 2.318 0 0 1-1.288-.412l-4.175-2.85a2.555 2.555 0 0 1-.787-.876A2.392 2.392 0 0 1 4 17.81V7.96c0-.374.08-.725.242-1.052l6.318 4.226Z"
  />
  <path
    fill="#F75858"
    d="M19.725 5.447A2.2 2.2 0 0 1 20 6.522v9.862c0 .409-.104.796-.313 1.163-.2.366-.48.658-.837.875l-7 4.3c-.399.243-.828.36-1.29.35V11.121l9.144-5.711.02.037Z"
  />
  <path
    fill="#8166E1"
    d="M20 16.384c0 .409-.104.796-.313 1.163-.2.366-.48.658-.837.875l-7 4.3c-.399.243-.828.36-1.29.35v-5.75l9.144-5.71c.01.01.296-.175.296-.175v4.947Z"
  />
  <path
    fill="#BDAAFF"
    d="M10.56 17.335v5.737h-.035a2.318 2.318 0 0 1-1.288-.412l-4.175-2.85a2.555 2.555 0 0 1-.787-.876A2.392 2.392 0 0 1 4 17.81v-4.84l6.56 4.366Z"
  />
  <path
    fill="#FFA43D"
    d="M4.242 6.907a2.285 2.285 0 0 1 .896-.985L12.1 1.646c.4-.25.834-.37 1.3-.362.467 0 .896.132 1.287.399l4.288 2.925c.312.216.554.484.728.8L10.56 11.12v.012L4.242 6.907Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#DC7157"
    d="M12 20.962c-.311 0-.644-.089-1-.289-2.433-1.355-5.367-1.666-7.8-1.122-1.4.311-2.2-.166-2.2-1.6V6.407c0-.911.433-1.69 1.5-2.067 3.122-1.089 7.211-.444 9.5 1.545 2.289-1.99 6.378-2.634 9.5-1.545 1.067.378 1.5 1.156 1.5 2.067V17.95c0 1.433-.8 1.911-2.2 1.6-2.433-.544-5.367-.233-7.8 1.122-.356.2-.689.29-1 .29Z"
  />
  <path
    fill="#EFEBDC"
    d="M3.381 3.403c2.962-1.043 7.07.074 8.623 2.479v12.876c-2.42-1.543-5.62-1.874-8.934-1.323V3.855c0-.202.121-.385.311-.452Z"
  />
  <path
    fill="#FBF2DF"
    d="M20.626 3.403c-2.962-1.043-7.07.074-8.622 2.479v12.876c2.42-1.543 5.62-1.874 8.934-1.323V3.855a.475.475 0 0 0-.312-.452Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#7DCC60"
    d="M12 22a9.804 9.804 0 0 1-5.013-1.337 10.084 10.084 0 0 1-3.65-3.65A9.812 9.812 0 0 1 2 12c0-1.808.446-3.48 1.337-5.013a9.987 9.987 0 0 1 3.65-3.637A9.727 9.727 0 0 1 12 2c1.808 0 3.48.45 5.012 1.35a9.891 9.891 0 0 1 3.638 3.637A9.727 9.727 0 0 1 22 12c0 1.808-.45 3.48-1.35 5.012a9.987 9.987 0 0 1-3.637 3.65C15.478 21.555 13.807 22 12 22Z"
  />
  <path
    fill="#fff"
    d="M17 8.462c.167.334.12.68-.137 1.038L12.15 16c-.292.4-.638.617-1.037.65-.4.033-.784-.13-1.15-.487l-2.638-2.55c-.317-.3-.425-.63-.325-.988a.988.988 0 0 1 .687-.7c.359-.117.696-.025 1.013.275l2.213 2.125 4.337-6.013c.258-.358.57-.508.938-.45.375.059.645.259.812.6Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#BEBEBE"
    d="M7 21c-.725 0-1.396-.18-2.013-.538a4.03 4.03 0 0 1-1.45-1.45A3.936 3.936 0 0 1 3 17V7c0-.725.18-1.392.538-2a4 4 0 0 1 1.45-1.462A3.936 3.936 0 0 1 7 3h10c.725 0 1.392.18 2 .538A3.971 3.971 0 0 1 20.462 5 3.87 3.87 0 0 1 21 7v10c0 .725-.18 1.396-.538 2.012A4 4 0 0 1 19 20.462 3.87 3.87 0 0 1 17 21H7Z"
  />
  <path
    fill="#9B9B9B"
    d="M8.294 18.402A7.183 7.183 0 0 0 12 19.4c1.34 0 2.575-.333 3.707-.998a7.466 7.466 0 0 0 2.695-2.696A7.184 7.184 0 0 0 19.4 12c0-1.34-.333-2.575-.998-3.707a7.466 7.466 0 0 0-2.696-2.695A7.184 7.184 0 0 0 12 4.6a7.18 7.18 0 0 0-3.706.998 7.466 7.466 0 0 0-2.696 2.696A7.184 7.184 0 0 0 4.6 12c0 1.34.333 2.575.998 3.707a7.466 7.466 0 0 0 2.696 2.695Z"
  />
  <path
    fill="#F4F4F4"
    d="M14.902 7.977a.916.916 0 0 1 1.121 1.121l-1.308 4.76c-.066.227-.169.41-.308.549-.14.14-.322.242-.55.308l-4.759 1.308a.916.916 0 0 1-1.121-1.121l1.308-4.76c.066-.227.169-.41.308-.549.14-.14.322-.242.55-.308l4.759-1.308Z"
  />
  <path
    fill="#F75858"
    d="m9.593 9.593 4.814 4.814c-.14.14-.322.242-.55.308l-4.759 1.308a.915.915 0 0 1-1.121-1.121l1.308-4.76c.066-.227.168-.41.308-.549Z"
  />
  <path
    fill="#EBEBEB"
    d="M6.907 12a.72.72 0 0 0-.72-.72H4.492a.72.72 0 0 0 0 1.44h1.697a.72.72 0 0 0 .719-.72Zm13.323 0a.72.72 0 0 0-.72-.72h-1.696a.72.72 0 0 0 0 1.44h1.697a.72.72 0 0 0 .719-.72ZM12 17.093a.72.72 0 0 0-.72.72v1.697a.72.72 0 0 0 1.44 0v-1.697a.72.72 0 0 0-.72-.72ZM12 3.77a.72.72 0 0 0-.72.72v1.697a.72.72 0 0 0 1.44 0V4.49a.72.72 0 0 0-.72-.72Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#EBEBEB"
    d="M18 3c.775 0 1.467.171 2.075.513a3.493 3.493 0 0 1 1.412 1.412C21.83 5.533 22 6.225 22 7v10c0 .775-.171 1.467-.513 2.075a3.575 3.575 0 0 1-1.412 1.425c-.608.333-1.3.5-2.075.5H6c-.775 0-1.467-.167-2.075-.5A3.66 3.66 0 0 1 2.5 19.075c-.333-.608-.5-1.3-.5-2.075V7c0-.775.167-1.467.5-2.075a3.575 3.575 0 0 1 1.425-1.412C4.533 3.17 5.225 3 6 3h12Z"
  />
  <rect width="8" height="5" x="5" y="6" fill="#B7B7B7" rx="1" />
  <rect width="4" height="5" x="15" y="6" fill="#B7B7B7" rx="1" />
  <rect width="14" height="5" x="5.014" y="13.081" fill="#B7B7B7" rx="1" />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#EBEBEB"
    d="M13 2.594c.267 0 .538.074.813.224.274.15.583.393.924.726l4.95 4.9c.542.541.813 1.091.813 1.65v7.45c0 .725-.179 1.396-.537 2.013a4.001 4.001 0 0 1-1.463 1.449 3.87 3.87 0 0 1-2 .538h-9c-.725 0-1.396-.18-2.013-.538a4.031 4.031 0 0 1-1.45-1.45 3.937 3.937 0 0 1-.537-2.012V6.594c0-.725.179-1.392.537-2a4.002 4.002 0 0 1 1.45-1.463A3.936 3.936 0 0 1 7.5 2.594H13Z"
  />
  <path
    fill="#F5F5F5"
    d="M13 2.594c.267 0 .537.075.813.225.274.15.583.391.925.725l4.95 4.9c.541.541.812 1.091.812 1.65h-4.787c-.834 0-1.496-.242-1.988-.725C13.242 8.877 13 8.215 13 7.38V2.594Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#DFDFDF"
    d="M13 2.594c.267 0 .538.074.813.224.274.15.583.393.924.726l4.95 4.9c.542.541.813 1.091.813 1.65v7.45c0 .725-.179 1.396-.537 2.013a4.001 4.001 0 0 1-1.463 1.449 3.87 3.87 0 0 1-2 .538h-9c-.725 0-1.396-.18-2.013-.538a4.031 4.031 0 0 1-1.45-1.45 3.937 3.937 0 0 1-.537-2.012V6.594c0-.725.179-1.392.537-2a4.002 4.002 0 0 1 1.45-1.463A3.936 3.936 0 0 1 7.5 2.594H13Z"
  />
  <path
    fill="#00C2A2"
    d="M9.818 8.606c-.25.142-.525.213-.825.213-.3 0-.58-.071-.838-.213-.25-.15-.45-.35-.6-.6a1.714 1.714 0 0 1-.212-.837c0-.3.07-.575.213-.825.15-.259.35-.459.6-.6a1.64 1.64 0 0 1 .837-.225c.3 0 .575.075.825.225.258.141.458.341.6.6.15.25.225.525.225.825 0 .3-.075.579-.225.837a1.6 1.6 0 0 1-.6.6Zm1.787 3.325c-.091.092-.22.138-.387.138h-4.45c-.167 0-.296-.046-.388-.138-.091-.091-.12-.22-.087-.387.075-.425.242-.804.5-1.138a2.729 2.729 0 0 1 2.2-1.088 2.729 2.729 0 0 1 2.2 1.088c.258.334.425.713.5 1.138.033.166.004.296-.088.387Z"
  />
  <path
    fill="#FDFDFD"
    d="M13 2.594c.267 0 .537.075.813.225.274.15.583.391.925.725l4.95 4.9c.541.541.812 1.091.812 1.65h-4.787c-.834 0-1.496-.242-1.988-.725C13.242 8.877 13 8.215 13 7.38V2.594Z"
  />
  <path
    fill="#9B9B9B"
    d="M17 14.005a.72.72 0 0 1 0 1.44H7a.72.72 0 0 1 0-1.44h10Zm-3.28 3.715A.72.72 0 0 0 13 17H7a.72.72 0 0 0 0 1.44h6a.72.72 0 0 0 .72-.72Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#2E9EFF"
    d="M22 9v4H2V7c0-2.35 1.65-4 4-4h2.637c2.988 0 2.75 2 4.613 2H18c2.35 0 4 1.65 4 4Z"
  />
  <path
    fill="#68C4FF"
    d="M18 21c2.35 0 4-1.65 4-4v-5a3 3 0 0 0-3-3H5a3 3 0 0 0-3 3v5c0 2.35 1.65 4 4 4h12Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <circle cx="12" cy="12" r="9" fill="#CDF3FF" />
  <path
    fill="#41CEF9"
    fill-rule="evenodd"
    d="M12 2c5.522 0 10 4.478 10 10s-4.478 10-10 10S2 17.522 2 12 6.478 2 12 2ZM9.172 13c.146 4.477 1.284 7 2.828 7 1.544 0 2.682-2.523 2.828-7H9.172Zm-5.108 0a7.994 7.994 0 0 0 4.313 6.134C7.686 17.622 7.261 15.549 7.174 13h-3.11Zm12.762 0c-.087 2.55-.512 4.622-1.204 6.134A7.994 7.994 0 0 0 19.936 13h-3.11Zm-8.45-8.135A7.995 7.995 0 0 0 4.065 11h3.11c.087-2.55.511-4.623 1.203-6.135ZM12.001 4c-1.544 0-2.682 2.523-2.828 7h5.656C14.682 6.523 13.544 4 12 4Zm3.622.865C16.314 6.377 16.74 8.45 16.826 11h3.11a7.995 7.995 0 0 0-4.314-6.135Z"
    clip-rule="evenodd"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#3C5CD8"
    d="M13 10.5h4.5c.708 0 1.325.142 1.85.425.533.283.942.692 1.225 1.225.283.525.425 1.142.425 1.85v2h-2v-2c0-.467-.134-.833-.4-1.1-.267-.266-.633-.4-1.1-.4H13V16h-2v-3.5H6.5c-.458 0-.825.134-1.1.4-.266.267-.4.633-.4 1.1v2H3v-2c0-.708.142-1.325.425-1.85a2.916 2.916 0 0 1 1.213-1.225c.533-.283 1.154-.425 1.862-.425H11V8h2v2.5Z"
  />
  <path
    fill="#6582F1"
    d="M9.012 2.513C8.671 2.854 8.5 3.35 8.5 4v2.5c0 .65.17 1.146.512 1.487.342.342.838.513 1.488.513h3c.65 0 1.146-.17 1.488-.513.341-.341.512-.837.512-1.487V4c0-.65-.17-1.146-.512-1.487C14.646 2.17 14.15 2 13.5 2h-3c-.65 0-1.146.17-1.488.513Zm.011 13.499c-.341.342-.512.838-.512 1.488V20c0 .65.17 1.146.512 1.488.342.341.838.512 1.488.512h3c.65 0 1.146-.17 1.487-.512.342-.342.513-.838.513-1.488v-2.5c0-.65-.171-1.146-.513-1.488-.341-.341-.837-.512-1.487-.512h-3c-.65 0-1.146.17-1.488.512Zm8 0c-.341.342-.512.838-.512 1.488V20c0 .65.17 1.146.512 1.488.342.341.838.512 1.488.512h3c.65 0 1.146-.17 1.487-.512.342-.342.513-.838.513-1.488v-2.5c0-.65-.171-1.146-.513-1.488-.341-.341-.837-.512-1.487-.512h-3c-.65 0-1.146.17-1.488.512Zm-16 0c-.341.342-.512.838-.512 1.488V20c0 .65.17 1.146.512 1.488.342.341.838.512 1.488.512h3c.65 0 1.146-.17 1.487-.512.342-.342.513-.838.513-1.488v-2.5c0-.65-.171-1.146-.513-1.488-.341-.341-.837-.512-1.487-.512h-3c-.65 0-1.146.17-1.488.512Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#FFD500"
    d="M12 1.62c4.566 0 7.777 3.256 7.777 7.556 0 4.166-3.244 5.722-3.244 7.955v.7l-.003.174H7.47a6.237 6.237 0 0 1-.004-.174v-.7c0-2.233-3.244-3.789-3.244-7.955 0-4.3 3.21-7.556 7.778-7.556Z"
  />
  <path
    fill="#BDBBBB"
    d="M16.53 18.005c-.048 2.655-1.781 4.317-4.53 4.317-2.75 0-4.483-1.662-4.532-4.317h9.063Z"
  />
  <path
    fill="#FFEFA0"
    d="M10.299 17.51h1v-7.19c0-1.098-.911-2.008-2.033-2.008-1.112 0-2.023.91-2.023 2.008 0 1.11.911 2.03 2.023 2.03h5.468c1.112 0 2.023-.92 2.023-2.03a2.026 2.026 0 0 0-2.023-2.008c-1.122 0-2.033.91-2.033 2.008v7.19h1v-7.19c0-.565.467-1.031 1.033-1.031a1.03 1.03 0 0 1 1.034 1.031c0 .577-.456 1.032-1.034 1.032H9.266a1.024 1.024 0 0 1-1.034-1.032 1.03 1.03 0 0 1 1.034-1.03c.566 0 1.033.466 1.033 1.031v7.19Z"
  />
  <path
    fill="#D9D9D9"
    d="M16.533 17.13v.701A6.254 6.254 0 0 1 16.425 19h-8.85a5.54 5.54 0 0 1-.108-1.169v-.7c0-.039-.004-.077-.006-.115h9.078c-.002.038-.006.076-.006.115Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#fff"
    d="M9.012 17.15c.917.533 1.913.8 2.988.8a5.771 5.771 0 0 0 2.975-.8 5.978 5.978 0 0 0 2.162-2.162c.534-.917.8-1.913.8-2.988a5.771 5.771 0 0 0-.8-2.975 5.978 5.978 0 0 0-2.162-2.163 5.77 5.77 0 0 0-2.975-.8c-1.075 0-2.07.267-2.988.8A5.978 5.978 0 0 0 6.85 9.025 5.77 5.77 0 0 0 6.05 12c0 1.075.267 2.07.8 2.988a5.978 5.978 0 0 0 2.162 2.162Z"
  />
  <path fill="#fff" d="M13 21V3h-2v18h2Z" />
  <path fill="#fff" d="M21 16.887 3 9.25V7.075l18 7.65v2.162Z" />
  <path
    fill="#FFC93C"
    d="M21 16.888V17c0 .775-.171 1.467-.513 2.075a3.576 3.576 0 0 1-1.412 1.425c-.608.333-1.3.5-2.075.5h-4v-3.135a5.714 5.714 0 0 0 1.975-.715 5.965 5.965 0 0 0 2.032-1.957L21 16.888Z"
  />
  <path
    fill="#FF8BC9"
    d="m3 9.25 3.212 1.362A5.979 5.979 0 0 0 6.05 12c0 1.075.266 2.07.8 2.987a5.981 5.981 0 0 0 2.163 2.163c.625.364 1.288.6 1.987.716V21H7c-.775 0-1.467-.167-2.075-.5A3.66 3.66 0 0 1 3.5 19.075c-.333-.608-.5-1.3-.5-2.075V9.25Z"
  />
  <path
    fill="#7DCC60"
    d="M11 6.146c-.7.115-1.362.352-1.987.716a5.966 5.966 0 0 0-2.008 1.915L3 7.075V7c0-.775.167-1.467.5-2.075a3.575 3.575 0 0 1 1.425-1.412C5.533 3.17 6.225 3 7 3h4v3.146ZM17 3c.775 0 1.467.171 2.075.513a3.493 3.493 0 0 1 1.412 1.412C20.83 5.533 21 6.225 21 7v7.725l-3.218-1.368c.101-.437.155-.89.155-1.357a5.77 5.77 0 0 0-.8-2.975 5.981 5.981 0 0 0-2.162-2.163A5.716 5.716 0 0 0 13 6.146V3h4Z"
  />
  <path
    fill="#2E9EFF"
    d="M10.012 15.425A3.92 3.92 0 0 0 12 15.95a3.85 3.85 0 0 0 1.975-.525 3.993 3.993 0 0 0 1.425-1.438c.358-.608.537-1.27.537-1.987a3.78 3.78 0 0 0-.537-1.975A3.895 3.895 0 0 0 13.975 8.6 3.78 3.78 0 0 0 12 8.063a3.85 3.85 0 0 0-1.988.537 3.99 3.99 0 0 0-1.437 1.425A3.85 3.85 0 0 0 8.05 12c0 .717.175 1.38.525 1.988a4.09 4.09 0 0 0 1.438 1.437Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#43D0FB"
    d="M10.84 19.23c-4.63 0-8.39-3.76-8.39-8.39 0-4.63 3.76-8.39 8.39-8.39 4.63 0 8.39 3.76 8.39 8.39 0 4.63-3.76 8.39-8.39 8.39Zm10.3 1.92c-.4.39-1.03.4-1.42 0l-3.99-4 1.41-1.41 4 3.99c.4.39.39 1.02 0 1.42Z"
  />
  <path
    fill="#CDF3FF"
    d="M17.23 10.84c0 3.53-2.86 6.39-6.39 6.39s-6.39-2.86-6.39-6.39 2.86-6.39 6.39-6.39 6.39 2.86 6.39 6.39Z"
  />
  <path
    fill="#1BA5CF"
    d="m21.35 19.58-3.77-3.77c-.5.67-1.09 1.27-1.77 1.77l3.77 3.77c.49.49 1.28.49 1.77 0s.49-1.28 0-1.77Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#EBEBEB"
    fill-rule="evenodd"
    d="M7.759 3h8.482c.805 0 1.47 0 2.01.044.563.046 1.08.145 1.565.392a4 4 0 0 1 1.748 1.748c.247.485.346 1.002.392 1.564C22 7.29 22 7.954 22 8.758v6.483c0 .805 0 1.47-.044 2.01-.046.563-.145 1.08-.392 1.565a4 4 0 0 1-1.748 1.748c-.485.247-1.002.346-1.564.392-.541.044-1.206.044-2.01.044H7.758c-.805 0-1.47 0-2.01-.044-.563-.046-1.08-.145-1.565-.392a4 4 0 0 1-1.748-1.748c-.247-.485-.346-1.002-.392-1.564C2 16.71 2 16.046 2 15.242V8.758c0-.805 0-1.47.044-2.01.046-.563.145-1.08.392-1.565a4 4 0 0 1 1.748-1.748c.485-.247 1.002-.346 1.564-.392C6.29 3 6.954 3 7.758 3Z"
    clip-rule="evenodd"
  />
  <path
    fill="#B7B7B7"
    d="M13.5 8a1 1 0 0 1 1-1H17a1 1 0 1 1 0 2h-2.5a1 1 0 0 1-1-1Zm0 4a1 1 0 0 1 1-1H17a1 1 0 1 1 0 2h-2.5a1 1 0 0 1-1-1Zm-7.502 4a1 1 0 0 1 1-1h10a1 1 0 1 1 0 2h-10a1 1 0 0 1-1-1Z"
  />
  <path
    fill="#9B9B9B"
    d="M6 8a1 1 0 0 1 1-1h3.5a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V8Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#9B9B9B"
    d="M12 22.033c-3.933 0-7.033-3.1-7.033-7.033V8.944c0-.555.444-1 1-1 .555 0 1 .445 1 1v6.045c0 2.822 2.21 5.044 5.033 5.044 2.822 0 5.033-2.21 5.033-5.033V6.989a2.997 2.997 0 0 0-3.022-3.022 2.997 2.997 0 0 0-3.022 3.022v7.989A1 1 0 0 0 12 15.988a1 1 0 0 0 1.011-1.01V8.944c0-.555.445-1 1-1 .556 0 1 .445 1 1v6.034A2.979 2.979 0 0 1 12 17.988a2.979 2.979 0 0 1-3.011-3.01v-7.99c0-2.81 2.211-5.021 5.022-5.021 2.811 0 5.022 2.21 5.022 5.022V15c0 3.933-3.1 7.033-7.033 7.033Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#FEBD08"
    d="M19.58 4.39c-1.53-1.52-3.9-1.5-5.4 0l-8.91 8.94c-1.01 1.01-1.42 1.59-1.67 2.93l-.53 3.3c-.18.91.44 1.53 1.37 1.37l3.31-.54c1.32-.21 1.9-.63 2.91-1.64l8.92-8.92c1.52-1.52 1.54-3.89 0-5.42v-.02Z"
  />
  <path
    fill="#FF928C"
    d="M19.58 9.82c1.52-1.52 1.54-3.89 0-5.42-1.53-1.52-3.9-1.5-5.4 0l-.41.41 5.41 5.41.4-.4Z"
  />
  <path
    fill="#D9D9D9"
    d="m13.77 4.813-1.415 1.414 5.41 5.41 1.414-1.415-5.41-5.409Z"
  />
  <path
    fill="#fff"
    d="m12.36 6.23-7.09 7.11c-1.01 1.01-1.42 1.59-1.67 2.93l-.53 3.3c-.09.45.02.83.26 1.08L15.05 8.93l-2.69-2.7Z"
    opacity=".5"
  />
  <path
    fill="#FFDDBC"
    d="M4.6 14.05c-.54.64-.81 1.22-1 2.21l-.09.55 3.67 3.67.57-.09c.99-.16 1.56-.43 2.2-.98L4.6 14.05Z"
  />
  <path
    fill="#4D4D4D"
    d="m3.51 16.81-.44 2.75c-.18.91.44 1.53 1.37 1.37l2.74-.45-3.67-3.67Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#2E9EFF"
    d="M12.725 20.288c-.367.716-.842 1.166-1.425 1.35-.583.191-1.15.12-1.7-.213-.55-.325-.954-.846-1.213-1.563L3.787 6.95c-.175-.492-.216-.958-.124-1.4.091-.45.291-.83.6-1.137a2.187 2.187 0 0 1 1.137-.6c.45-.092.92-.05 1.412.125l12.913 4.6c.717.258 1.237.662 1.563 1.212.333.542.404 1.104.212 1.688-.183.583-.633 1.058-1.35 1.425l-4.925 2.512-2.5 4.913Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#FFD400"
    d="M12 12v9H7a3.87 3.87 0 0 1-2-.537 4.003 4.003 0 0 1-1.463-1.45A3.936 3.936 0 0 1 3 17v-5h9Z"
  />
  <path
    fill="#FFA43D"
    d="M12 12H9.605a2.167 2.167 0 1 1-4.232 0H3V7c0-.725.179-1.392.537-2a4.002 4.002 0 0 1 1.45-1.463A3.936 3.936 0 0 1 7 3h5v9Z"
  />
  <path
    fill="#F75858"
    d="M12 12V9.605a2.167 2.167 0 1 1 0-4.232V3h5a3.87 3.87 0 0 1 2 .537 4.002 4.002 0 0 1 1.463 1.45c.359.617.538 1.288.538 2.013v5h-9Z"
  />
  <path
    fill="#FF8082"
    d="M12 12h2.395a2.167 2.167 0 1 1 4.232 0H21v5a3.87 3.87 0 0 1-.537 2 4.002 4.002 0 0 1-1.45 1.463A3.937 3.937 0 0 1 17 21h-5v-9Z"
  />
  <circle
    cx="12.464"
    cy="16.5"
    r="2.167"
    fill="#FFC93C"
    transform="rotate(-90 12.464 16.5)"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <circle cx="12" cy="12" r="10" fill="#36DEC3" fill-opacity=".25" />
  <circle cx="12" cy="12" r="6" fill="#36DEC3" fill-opacity=".25" />
  <path
    fill="#36DEC3"
    d="M12 22a9.804 9.804 0 0 1-5.013-1.337 10.084 10.084 0 0 1-3.65-3.65A9.812 9.812 0 0 1 2 12c0-1.808.446-3.48 1.337-5.013a9.987 9.987 0 0 1 3.65-3.637A9.727 9.727 0 0 1 12 2c.275 0 .508.1.7.3.2.192.3.425.3.7a.97.97 0 0 1-.3.712A.953.953 0 0 1 12 4c-1.45 0-2.787.358-4.013 1.075a8.001 8.001 0 0 0-2.912 2.912A7.805 7.805 0 0 0 4 12a7.8 7.8 0 0 0 1.075 4.012 8.001 8.001 0 0 0 2.912 2.913A7.804 7.804 0 0 0 12 20a7.8 7.8 0 0 0 4.012-1.075 8.002 8.002 0 0 0 2.913-2.913A7.804 7.804 0 0 0 20 12a7.79 7.79 0 0 0-.45-2.637 7.74 7.74 0 0 0-1.262-2.3c-.292-.375-.367-.738-.226-1.088.15-.35.405-.563.763-.637.367-.084.68.033.938.35a9.746 9.746 0 0 1 1.65 2.925A9.817 9.817 0 0 1 22 12c0 1.808-.45 3.48-1.35 5.012a9.987 9.987 0 0 1-3.637 3.65C15.478 21.555 13.807 22 12 22Z"
  />
  <path
    fill="#2EC9B0"
    d="M8.988 17.2c.916.533 1.92.8 3.012.8a5.884 5.884 0 0 0 3.012-.8 6.015 6.015 0 0 0 2.175-2.188c.542-.916.813-1.92.813-3.012a5.8 5.8 0 0 0-.288-1.825 5.68 5.68 0 0 0-.8-1.625c-.2-.283-.487-.404-.862-.363a.971.971 0 0 0-.8.55c-.167.317-.113.705.162 1.163.392.642.588 1.342.588 2.1 0 .733-.18 1.404-.537 2.012a4.03 4.03 0 0 1-1.45 1.45A3.893 3.893 0 0 1 12 16c-.733 0-1.404-.18-2.012-.537a4.03 4.03 0 0 1-1.45-1.45A3.892 3.892 0 0 1 8 12c0-.733.18-1.404.537-2.012a4.03 4.03 0 0 1 1.45-1.45A3.892 3.892 0 0 1 12 8a.953.953 0 0 0 .7-.287c.2-.2.3-.438.3-.713 0-.275-.1-.508-.3-.7A.933.933 0 0 0 12 6c-1.092 0-2.096.27-3.012.813A6.016 6.016 0 0 0 6.8 8.988C6.267 9.904 6 10.908 6 12s.267 2.096.8 3.012A6.115 6.115 0 0 0 8.988 17.2Z"
  />
  <path
    fill="#00B194"
    d="M12 .925c.275 0 .508.1.7.3.2.191.3.425.3.7v8.35l.112.069c.255.167.46.386.613.656.183.3.275.633.275 1s-.092.704-.275 1.013c-.175.3-.417.541-.725.724-.3.175-.633.263-1 .263s-.704-.088-1.013-.263a2.155 2.155 0 0 1-.724-.724A2.017 2.017 0 0 1 10 12c0-.367.088-.7.263-1a2.02 2.02 0 0 1 .724-.725l.013-.007V1.925a.95.95 0 0 1 .287-.7c.2-.2.438-.3.713-.3Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#4D6FE6"
    d="M12 2.75c.248 0 .492.059.712.172l6 3.077A1.6 1.6 0 0 1 19.6 7.42v4.067c0 4.328-2.861 8.043-6.9 9.287a2.362 2.362 0 0 1-1.4 0c-4.039-1.244-6.9-4.959-6.9-9.287V7.42c0-.6.338-1.148.888-1.42l6-3.077A1.55 1.55 0 0 1 12 2.75Z"
  />
  <path
    fill="#86A0F2"
    d="M12 5.154 7.4 7.512v3.975c0 3.084 1.862 5.812 4.6 7.05V5.154Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#5856D6"
    d="M9 21c-1.217 0-2.28-.242-3.188-.725a5.14 5.14 0 0 1-2.087-2.087C3.242 17.279 3 16.216 3 15V9c0-1.217.242-2.275.725-3.175a5.04 5.04 0 0 1 2.087-2.088C6.721 3.246 7.784 3 9 3h6c1.217 0 2.275.246 3.175.737a4.946 4.946 0 0 1 2.087 2.088c.492.9.738 1.958.738 3.175v6c0 1.217-.246 2.28-.738 3.188a5.04 5.04 0 0 1-2.087 2.087c-.9.483-1.958.725-3.175.725H9Z"
  />
  <path
    fill="#FFC93C"
    d="M9.337 16.1a.84.84 0 0 0 .513-.038l2.15-.875 2.137.876a.806.806 0 0 0 .5.037.697.697 0 0 0 .388-.275.808.808 0 0 0 .125-.487l-.175-2.3 1.5-1.776a.686.686 0 0 0 .188-.462.692.692 0 0 0-.138-.463.705.705 0 0 0-.425-.262l-2.25-.55-1.225-1.963a.768.768 0 0 0-.387-.325.666.666 0 0 0-.476 0 .777.777 0 0 0-.375.325L10.15 9.525l-2.25.55a.704.704 0 0 0-.425.263.75.75 0 0 0-.15.462c.008.167.075.32.2.462l1.5 1.763-.188 2.313a.807.807 0 0 0 .125.487.696.696 0 0 0 .375.275Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#FFA43D"
    d="M6.775 20.725a2.053 2.053 0 0 1-1.275.25 1.725 1.725 0 0 1-1.075-.55c-.283-.308-.425-.7-.425-1.175V7c0-.775.167-1.467.5-2.075a3.575 3.575 0 0 1 1.425-1.412C6.533 3.17 7.225 3 8 3h8c.775 0 1.467.17 2.075.513.608.333 1.08.804 1.413 1.412.341.608.512 1.3.512 2.075v12.25c0 .442-.142.82-.425 1.137-.275.309-.63.5-1.063.575-.433.076-.862-.004-1.287-.237L12 17.837l-5.225 2.888Z"
  />
  <path
    fill="#FBF4EB"
    d="M14.5 14.75a.78.78 0 0 1-.475-.025L12 13.887l-2.038.838a.78.78 0 0 1-.475.025.721.721 0 0 1-.362-.262.757.757 0 0 1-.113-.45l.175-2.2-1.412-1.663a.71.71 0 0 1-.188-.438.7.7 0 0 1 .138-.425.72.72 0 0 1 .4-.25l2.125-.524 1.175-1.85a.672.672 0 0 1 .35-.3.6.6 0 0 1 .45 0c.15.05.275.15.375.3l1.138 1.85 2.137.524a.66.66 0 0 1 .387.25c.1.126.146.267.138.426a.65.65 0 0 1-.175.437l-1.413 1.675.163 2.188a.71.71 0 0 1-.125.45.657.657 0 0 1-.35.262Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#EBEBEB"
    d="M13 2.594c.267 0 .538.074.813.224.274.15.583.393.924.726l4.95 4.9c.542.541.813 1.091.813 1.65v7.45c0 .725-.179 1.396-.537 2.013a4.001 4.001 0 0 1-1.463 1.449 3.87 3.87 0 0 1-2 .538h-9c-.725 0-1.396-.18-2.013-.538a4.031 4.031 0 0 1-1.45-1.45 3.937 3.937 0 0 1-.537-2.012V6.594c0-.725.179-1.392.537-2a4.002 4.002 0 0 1 1.45-1.463A3.936 3.936 0 0 1 7.5 2.594H13Z"
  />
  <path
    fill="#9B9B9B"
    d="M14.696 16.734c.374 0 .677.323.677.72 0 .398-.303.72-.677.72H7.304c-.374 0-.677-.322-.677-.72 0-.397.303-.72.677-.72h7.392Zm2-3.004c.374 0 .677.323.677.72 0 .398-.303.72-.677.72H7.304c-.374 0-.677-.322-.677-.72 0-.397.303-.72.677-.72h9.392Z"
  />
  <path
    fill="#F5F5F5"
    d="M13 2.594c.267 0 .537.075.813.225.274.15.583.391.925.725l4.95 4.9c.541.541.812 1.091.812 1.65h-4.787c-.834 0-1.496-.242-1.988-.725C13.242 8.877 13 8.215 13 7.38V2.594Z"
  />
</svg>
`,`<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  fill="none"
  viewBox="0 0 24 24"
>
  <path
    fill="#FFDE83"
    d="M17.287 3.008c.66.036 1.256.2 1.788.492A3.66 3.66 0 0 1 20.5 4.925c.25.456.406.96.469 1.51A4.73 4.73 0 0 1 21 7v7.942h-4.993V21H7c-.775 0-1.467-.167-2.075-.5A3.66 3.66 0 0 1 3.5 19.075c-.333-.608-.5-1.3-.5-2.075V7c0-.775.167-1.467.5-2.075a3.575 3.575 0 0 1 1.425-1.412C5.533 3.17 6.225 3 7 3h10l.287.008Z"
  />
  <path
    fill="#EEC35F"
    d="M15.104 15a1 1 0 1 1 0 2H7.015a1 1 0 1 1 0-2h8.089Zm-3.089-4a1 1 0 1 1 0 2h-5a1 1 0 1 1 0-2h5Zm4.907-4a1 1 0 1 1 0 2H7.015a1 1 0 1 1 0-2h9.907Z"
  />
  <path
    fill="#2E9EFF"
    d="M14.9 22.012a1.58 1.58 0 0 1-.725-.887l-2.825-7.912c-.1-.3-.125-.588-.075-.863.058-.275.183-.508.375-.7.192-.192.425-.313.7-.363.275-.058.563-.033.863.075l7.912 2.825c.4.142.696.388.887.738.192.342.234.696.125 1.063-.1.358-.354.641-.762.85l-3.025 1.524-1.513 3.013c-.208.408-.495.663-.862.762a1.358 1.358 0 0 1-1.075-.125Z"
  />
</svg>
`].map(e=>`data:image/svg+xml,${encodeURIComponent(e)}`);var Re=e(a(),1),ze={".svg":`image/svg+xml`,".png":`image/png`,".jpg":`image/jpeg`,".jpeg":`image/jpeg`,".webp":`image/webp`,".gif":`image/gif`,".avif":`image/avif`};async function Be(e,t,n){let r=Ve(e);if(r==null)return null;try{let e={path:r,hostId:t},i=await n.fetchQuery({queryFn:({signal:t})=>x(`read-file-binary`,{params:e,signal:t}),queryKey:O(`read-file-binary`,e),retry:!1,gcTime:1/0,staleTime:D.INFINITE});return i.contentsBase64?`data:${He(r)};base64,${i.contentsBase64}`:null}catch(e){return P.warning(`Failed to inline local image`,{safe:{},sensitive:{error:e,resolvedImagePath:r}}),null}}function Ve(e){if(e==null)return null;let t=e.trim();if(t.length===0)return null;let n=t.toLowerCase();if(n.startsWith(`data:`)||n.startsWith(`http:`)||n.startsWith(`https:`)||n.startsWith(`file:`)||n.startsWith(`vscode-resource:`)||n.startsWith(`vscode-webview:`)||n.startsWith(`vscode-file:`))return null;let r=d(t);return u(r)?r:null}function He(e){return ze[Re.default.extname(e).toLowerCase()]??`application/octet-stream`}var Z=[`plugins`],Ue=`openai-curated-marketplaces-hidden`,We=4,Ge=[],Ke=[],qe=[],Je=[],Ye=[],Q=[],Xe=[_],Ze=[_,h],Qe=`computer-use`,$e=`.tmp/marketplaces/openai-internal-testing`,et=[`datadog`,`statsig`],tt=new Set([`codex-official`,m,_,h,`openai-primary-runtime`].map(Nt)),nt=k(E,({buildFlavor:e,hostId:t,installSuggestionPluginNames:n,isOpenAICuratedRemoteMarketplaceEnabled:r,marketplaceKinds:i,roots:a,shouldHideOpenAICuratedMarketplaces:o},{queryClient:s})=>{let c=yt({isOpenAICuratedRemoteMarketplaceEnabled:r,shouldHideOpenAICuratedMarketplaces:o}),l=n==null?_t(t,a,i,r,o):mt(t,a,qe,n,r,o);return{queryKey:l,queryFn:async()=>{if(n!=null){let e=await L(`send-cli-request-for-host`,{hostId:t,method:`plugin/installed`,params:{...a.length>0?{cwds:a}:{},installSuggestionPluginNames:n}}),r=bt(e.marketplaces,c),i=Bt(r,s.getQueryData(l)?.plugins);return{featuredPluginIds:Ge,marketplaceLoadErrors:e.marketplaceLoadErrors,marketplaces:Ft(r),plugins:await It({hostId:t,plugins:i,queryClient:s})}}let r=await L(`list-plugins`,i==null?{hostId:t,...a.length>0?{cwds:a}:{}}:{hostId:t,...a.length>0?{cwds:a}:{},marketplaceKinds:i}),o=bt(r.marketplaces,c),u=Bt(o,s.getQueryData(l)?.plugins),d=e==null?u:Mt({buildFlavor:e,plugins:u}),f=ot(r.featuredPluginIds).filter(e=>!c.some(t=>e.endsWith(`@${t}`)));return{featuredPluginIds:e==null?f:jt({buildFlavor:e,featuredPluginIds:f}),marketplaceLoadErrors:r.marketplaceLoadErrors,marketplaces:Ft(o),plugins:await It({hostId:t,plugins:d,queryClient:s})}},staleTime:D.SIX_HOURS,gcTime:1/0}});k(E,({hostId:e,marketplaceKind:t},{queryClient:n})=>{let r=[...Z,`marketplace-kind`,e,t];return{queryKey:r,queryFn:async()=>Bt((await L(`list-plugins`,{hostId:e,marketplaceKinds:[t]})).marketplaces,n.getQueryData(r)),staleTime:D.SIX_HOURS}});function rt(e){return{...Vt(e.summary),description:e.summary.interface?.shortDescription??e.description??null,displayName:e.summary.interface?.displayName??null,marketplaceDisplayName:null,marketplaceName:e.marketplaceName,plugin:e.summary,keywords:e.summary.keywords,...ut({marketplaceName:e.marketplaceName,marketplacePath:e.marketplacePath})}}function it({marketplacePath:e,remoteMarketplaceName:t}){if(e!=null&&t!=null)throw Error(`plugin marketplace request requires one marketplace source`);if(e!=null)return{marketplacePath:e};if(t!=null)return{remoteMarketplaceName:t};throw Error(`plugin marketplace request requires a marketplace source`)}function at(e){return e.marketplacePath??`remote:${e.remoteMarketplaceName}`}function ot(e){return e.filter(e=>{let t=At(e).toLowerCase();return!et.some(e=>t===e||t.startsWith(`${e}-`))})}function st(e){return e.marketplacePath==null?ct(e.plugin):e.plugin.name}function ct(e){if(e.remotePluginId==null)throw Error(`remote plugin ${e.id} is missing remotePluginId`);return e.remotePluginId}function lt(e){return{...it(e),pluginName:st(e)}}function ut({marketplaceName:e,marketplacePath:t}){return t==null?{marketplacePath:null,remoteMarketplaceName:e}:{marketplacePath:t,remoteMarketplaceName:null}}function dt(e,t,n){let r=(0,J.c)(65),i;r[0]===e?i=r[1]:(i={hostId:e},r[0]=e,r[1]=i);let a=Te(i)&&(n?.enabled??!0),o=n?.additionalMarketplaceKinds??qe,s=n?.installSuggestionPluginNames??null,c=ie(`4218407052`),l=se(e)?.authMethod??null,u;r[2]===l?u=r[3]:(u=Le(l),r[2]=l,r[3]=u);let d=u,f=n?.includeRemoteCatalog??!0,p=!c,m;r[4]!==o||r[5]!==f||r[6]!==p?(m=gt({additionalMarketplaceKinds:o,includeRemoteCatalog:f,includeVerticalCatalog:p}),r[4]=o,r[5]=f,r[6]=p,r[7]=m):m=r[7];let h=m,g=oe(),_=F(ee),v;r[8]!==e||r[9]!==_?(v=_.includes(e),r[8]=e,r[9]=_,r[10]=v):v=r[10];let y=v,b=t===void 0,x;r[11]===e?x=r[12]:(x={hostId:e},r[11]=e,r[12]=x);let C=a&&y&&b,w;r[13]===C?w=r[14]:(w={enabled:C},r[13]=C,r[14]=w);let T=S(ae,x,w),E=ue(e),D=a&&y,O;r[15]!==e||r[16]!==D?(O={enabled:D,hostId:e},r[15]=e,r[16]=D,r[17]=O):O=r[17];let k=ge(O),A;r[18]===e?A=r[19]:(A={hostId:e},r[18]=e,r[19]=A);let j=Se(A),M;r[20]===e?M=r[21]:(M={hostId:e},r[20]=e,r[21]=M);let N=be(M),P=T.data?.roots,I;r[22]!==E||r[23]!==e||r[24]!==t||r[25]!==P?(I=Lt({codexHome:E,hostId:e,rootsOverrideCwd:t,workspaceRoots:P}),r[22]=E,r[23]=e,r[24]=t,r[25]=P,r[26]=I):I=r[26];let L=I,R=a&&y&&(t!==void 0||T.isFetched),te;r[27]===Symbol.for(`react.memo_cache_sentinel`)?(te=`prod`,r[27]=te):te=r[27];let ne=te,re;r[28]!==e||r[29]!==s||r[30]!==c||r[31]!==h||r[32]!==L||r[33]!==d?(re={buildFlavor:ne,hostId:e,installSuggestionPluginNames:s,isOpenAICuratedRemoteMarketplaceEnabled:c,marketplaceKinds:h,roots:L,shouldHideOpenAICuratedMarketplaces:d},r[28]=e,r[29]=s,r[30]=c,r[31]=h,r[32]=L,r[33]=d,r[34]=re):re=r[34];let z;r[35]===R?z=r[36]:(z={enabled:R},r[35]=R,r[36]=z);let B=S(nt,re,z);if(!a||!y){let e;return r[37]===Symbol.for(`react.memo_cache_sentinel`)?(e={availablePlugins:Q,featuredPluginIds:Ge,installedPlugins:Q,marketplaceLoadErrors:Ke,marketplaces:Ye,errorMessage:null,isLoading:!1,isFetching:!1,refetch:pt,forceReload:ft},r[37]=e):e=r[37],e}let V,H,U,W;r[38]!==k.available||r[39]!==N.available||r[40]!==j.available||r[41]!==B.data?.featuredPluginIds||r[42]!==B.data?.plugins?(V={isComputerUseAvailable:k.available,isExternalBrowserUseAvailable:N.available,isInAppBrowserUseAvailable:j.available},H=B.data?.plugins??Q,U=St({plugins:H,...V}),W=xt({featuredPluginIds:B.data?.featuredPluginIds??Ge,...V}),r[38]=k.available,r[39]=N.available,r[40]=j.available,r[41]=B.data?.featuredPluginIds,r[42]=B.data?.plugins,r[43]=V,r[44]=H,r[45]=U,r[46]=W):(V=r[43],H=r[44],U=r[45],W=r[46]);let G;r[47]===H?G=r[48]:(G=Ct(H),r[47]=H,r[48]=G);let ce=B.data?.marketplaceLoadErrors??Ke,le=B.data?.marketplaces??Ye,de=B.error?String(B.error.message):null,fe=b&&T.isLoading||B.isLoading||j.isLoading||N.isLoading||k.isLoading,pe=b&&T.isFetching||B.isFetching||k.isFetching,K;r[49]!==V||r[50]!==B?(K=async()=>{let e=(await B.refetch()).data?.plugins??Q;return{availablePlugins:St({plugins:e,...V}),installedPlugins:Ct(e)}},r[49]=V,r[50]=B,r[51]=K):K=r[51];let q;r[52]===g?q=r[53]:(q=()=>g(Z),r[52]=g,r[53]=q);let Y;return r[54]!==U||r[55]!==W||r[56]!==G||r[57]!==ce||r[58]!==le||r[59]!==de||r[60]!==fe||r[61]!==pe||r[62]!==K||r[63]!==q?(Y={availablePlugins:U,featuredPluginIds:W,installedPlugins:G,marketplaceLoadErrors:ce,marketplaces:le,errorMessage:de,isLoading:fe,isFetching:pe,refetch:K,forceReload:q},r[54]=U,r[55]=W,r[56]=G,r[57]=ce,r[58]=le,r[59]=de,r[60]=fe,r[61]=pe,r[62]=K,r[63]=q,r[64]=Y):Y=r[64],Y}async function ft(){}async function pt(){return{availablePlugins:Q,installedPlugins:Q}}function mt(e,t,n=qe,r=null,i=!1,a=!1){return r==null?_t(e,t,gt({additionalMarketplaceKinds:n,includeRemoteCatalog:!0,includeVerticalCatalog:!i}),i,a):ht([...Z,e,t,`installed`,r,`curated-marketplace`,vt(i)],a)}function ht(e,t){return t?[...e,Ue]:e}function gt({additionalMarketplaceKinds:e,includeRemoteCatalog:t,includeVerticalCatalog:n}){return t&&!n&&e.length===0?null:n?[`local`,`vertical`,...e]:[`local`,...e]}function _t(e,t,n,r,i){let a=vt(r);return ht(n==null?[...Z,e,t,`curated-marketplace`,a]:[...Z,e,t,`marketplace-kinds`,n,`curated-marketplace`,a],i)}function vt(e){return e?h:_}function yt({isOpenAICuratedRemoteMarketplaceEnabled:e,shouldHideOpenAICuratedMarketplaces:t}){return t?Ze:e?Xe:Je}function bt(e,t){return t.length===0?e:e.filter(e=>!t.includes(e.name))}function xt({featuredPluginIds:e,...t}){return e.filter(e=>Et(e,t))}function St({plugins:e,...t}){return e.filter(e=>Et(e.plugin.id,t))}function Ct(e){return e.filter(e=>e.plugin.installed)}function wt({enabled:e,hostId:t,installed:n,pluginId:r,queryClient:i}){i.setQueriesData({queryKey:[...Z,t]},t=>{if(t==null)return t;let i=Tt({enabled:e,installed:n,pluginId:r,plugins:t.plugins});return i===t.plugins?void 0:{...t,plugins:i}}),i.setQueriesData({queryKey:[...Z,`marketplace-kind`,t]},t=>{if(t==null)return;let i=Tt({enabled:e,installed:n,pluginId:r,plugins:t});return i===t?void 0:i}),i.setQueriesData({queryKey:[...Z,`detail`,t]},t=>{if(!(t==null||t.summary.id!==r||t.summary.installed===n&&t.summary.enabled===e))return{...t,summary:{...t.summary,enabled:e,installed:n}}})}function Tt({enabled:e,installed:t,pluginId:n,plugins:r}){return r.some(r=>r.plugin.id===n&&(r.plugin.installed!==t||r.plugin.enabled!==e))?r.map(r=>r.plugin.id===n?{...r,plugin:{...r.plugin,enabled:e,installed:t}}:r):r}function Et(e,{isComputerUseAvailable:t,isExternalBrowserUseAvailable:n,isInAppBrowserUseAvailable:r}){return!(!r&&Dt(e)||!n&&Ot(e)||!t&&kt(e))}function Dt(e){let t=At(e);return t===`browser`||t===`browser-use`}function Ot(e){let t=At(e);return t===`chrome`||t===`chrome-dev`||t===`chrome-internal`}function kt(e){return At(e)===Qe}function At(e){return e.split(`@`)[0]}function jt({buildFlavor:e,featuredPluginIds:t}){let n=f(e);return t.filter(e=>{let t=p(e);return t==null?!0:!o(t)||t===n})}function Mt({buildFlavor:e,plugins:t}){let n=f(e);return t.filter(e=>!o(e.marketplaceName)||e.marketplaceName===n)}function Nt(e){return e.trim().toLowerCase().replace(/[^a-z0-9]+/g,`-`).replace(/^-+|-+$/g,``)}function Pt(e){return tt.has(Nt(e))}function Ft(e){let t=new Map;for(let n of e){let e=`${n.name}\u0000${n.path??``}`;if(t.has(e))continue;let r=Pt(n.name);t.set(e,{displayName:n.interface?.displayName??null,isBuiltIn:r,name:n.name,path:n.path,pluginCount:n.plugins.length})}return Array.from(t.values()).sort((e,t)=>{let n=e.displayName?.trim()||e.name,r=t.displayName?.trim()||t.name;return n.localeCompare(r)})}async function It({hostId:e,plugins:t,queryClient:n}){let r=[...t];async function i(a){let o=t[a];if(o==null)return;let[s,c,l]=await Promise.all([Be(o.composerIconPath,e,n),Be(o.logoPath,e,n),Be(o.logoDarkPath,e,n)]);(s!=null||c!=null||l!=null)&&(r[a]={...o,composerIconPath:s??o.composerIconPath,logoDarkPath:l??o.logoDarkPath,logoPath:c??o.logoPath,plugin:o.plugin.interface?{...o.plugin,interface:{...o.plugin.interface,composerIcon:s??o.plugin.interface.composerIcon,logo:c??o.plugin.interface.logo,...l==null?{}:{logoDark:l}}}:o.plugin}),await i(a+We)}return await Promise.all(Array.from({length:Math.min(We,t.length)},(e,t)=>i(t))),r}function Lt({codexHome:e,hostId:t,rootsOverrideCwd:n,workspaceRoots:r}){let i=t===`local`&&e!=null?I(e,$e):null;return Rt([...typeof n==`string`?[n]:n??r??[],...i==null?[]:[i]],e)}function Rt(e,t){return Array.from(new Set(e.map(e=>e.trim()).filter(e=>u(e)&&zt(e,t))))}function zt(e,t){return t==null?!0:s(t)||c(t)?s(e)||c(e):e.startsWith(`/`)&&!e.startsWith(`//`)}function Bt(e,t=Q){let n=new Map,r=new Set;for(let t of e)for(let e of t.plugins){e.installed||r.add(e.id);let i={...Vt(e),description:e.interface?.shortDescription??null,displayName:e.interface?.displayName??null,marketplaceDisplayName:t.interface?.displayName??null,marketplaceName:t.name,plugin:e,keywords:e.keywords,...ut({marketplaceName:t.name,marketplacePath:t.path})},a=n.get(e.id);if(a==null){n.set(e.id,i);continue}let o=a;(a.plugin.installed&&!e.installed||a.plugin.installed===e.installed&&a.plugin.interface==null&&e.interface!=null)&&(o=i),a.plugin.installed&&!e.installed&&n.delete(e.id);let s=null;if(a.plugin.installed?s=a.plugin:e.installed&&(s=e),s==null){n.set(e.id,o);continue}n.set(e.id,{...o,plugin:{...o.plugin,enabled:s.enabled,installed:!0,installPolicy:s.installPolicy,localVersion:s.localVersion,remotePluginId:o.plugin.remotePluginId??s.remotePluginId}})}let i=new Map(t.map(e=>[e.plugin.id,e])),a=!1,o=Array.from(n.values()).map(e=>{let t=i.get(e.plugin.id);return!e.plugin.installed||t==null||r.has(e.plugin.id)?e:(a||=e.marketplaceName!==t.marketplaceName||e.marketplacePath!==t.marketplacePath,{...t,plugin:{...e.plugin,id:t.plugin.id,interface:t.plugin.interface??e.plugin.interface,keywords:t.plugin.keywords??e.plugin.keywords,name:t.plugin.name,remotePluginId:t.plugin.remotePluginId??e.plugin.remotePluginId,shareContext:t.plugin.shareContext??e.plugin.shareContext,source:t.plugin.source}})});if(!a)return o;let s=new Map(o.map(e=>[e.plugin.id,e]));return[...t.flatMap(e=>{let t=s.get(e.plugin.id);return s.delete(e.plugin.id),t==null?[]:[t]}),...s.values()]}function Vt(e,t){let n=e.interface,r=n?.composerIcon??n?.composerIconUrl,i=n?.logo??n?.logoUrl,a=n?.logoDark??n?.logoUrlDark,o=Pe(e),s=i!=null||a!=null,c=r??(s?null:o),l=s?i??a??null:o??t?.logoUrl??t?.logoUrlDark??null;return{composerIconPath:c,logoDarkPath:s?a??null:o??t?.logoUrlDark??t?.logoUrl??null,logoPath:l}}var $=i(),Ht=e=>(0,$.jsxs)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`path`,{d:`M4.65723 15.3333C4.65723 13.8605 5.85326 12.6666 7.32862 12.6666H10V15.3333C10 16.806 8.80398 18 7.32862 18C5.85326 18 4.65723 16.806 4.65723 15.3333Z`,fill:`#24CB71`}),(0,$.jsx)(`path`,{d:`M10 2V7.33333H12.6714C14.1468 7.33333 15.3428 6.13941 15.3428 4.66666C15.3428 3.19392 14.1468 2 12.6714 2H10Z`,fill:`#FF7237`}),(0,$.jsx)(`path`,{d:`M12.6489 12.6666C14.1243 12.6666 15.3203 11.4727 15.3203 9.99998C15.3203 8.52722 14.1243 7.33331 12.6489 7.33331C11.1736 7.33331 9.97754 8.52722 9.97754 9.99998C9.97754 11.4727 11.1736 12.6666 12.6489 12.6666Z`,fill:`#00B6FF`}),(0,$.jsx)(`path`,{d:`M4.65723 4.66666C4.65723 6.13941 5.85326 7.33333 7.32862 7.33333H10V2H7.32862C5.85326 2 4.65723 3.19392 4.65723 4.66666Z`,fill:`#FF3737`}),(0,$.jsx)(`path`,{d:`M4.65723 9.99998C4.65723 11.4727 5.85326 12.6666 7.32862 12.6666H10V7.33331H7.32862C5.85326 7.33331 4.65723 8.52723 4.65723 9.99998Z`,fill:`#874FFF`})]}),Ut=e=>(0,$.jsxs)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`path`,{d:`M3.6001 3.6C3.6001 2.71634 4.31644 2 5.2001 2H11.6001L16.4001 6.8V16.4C16.4001 17.2837 15.6838 18 14.8001 18H5.2001C4.31644 18 3.6001 17.2837 3.6001 16.4V3.6Z`,fill:`#34A853`}),(0,$.jsx)(`path`,{d:`M11.6001 2L16.4001 6.8H13.2001C12.3164 6.8 11.6001 6.08366 11.6001 5.2V2Z`,fill:`#188038`}),(0,$.jsx)(`path`,{d:`M6.40049 8.79999C6.17957 8.79999 6.00049 8.97907 6.00049 9.19999V14.4C6.00049 14.6209 6.17957 14.8 6.40049 14.8H13.6005C13.8214 14.8 14.0005 14.6209 14.0005 14.4V9.19999C14.0005 8.97907 13.8214 8.79999 13.6005 8.79999H6.40049ZM9.40049 13.6H7.20049V12.2L9.40049 12.2V13.6ZM9.40049 11.4H7.20049V9.99999H9.40049V11.4ZM12.8005 13.6H10.6005V12.2L12.8005 12.2V13.6ZM12.8005 11.4H10.6005V9.99999H12.8005V11.4Z`,fill:`white`})]}),Wt=e=>(0,$.jsxs)(`svg`,{width:192,height:192,viewBox:`0 0 192 192`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`mask`,{id:`google-docs-2026-mask0_37242_8762`,style:{maskType:`alpha`},maskUnits:`userSpaceOnUse`,x:32,y:8,width:128,height:176,children:(0,$.jsx)(`path`,{d:`M130.334 184L61.6 184C52.6565 184 48.1848 184 44.6375 182.596C39.5029 180.563 35.4374 176.497 33.4045 171.362C32 167.815 32 163.343 32 154.4L32 37.6C32 28.6565 32 24.1848 33.4045 20.6375C35.4374 15.5029 39.5029 11.4374 44.6375 9.40447C48.1848 8 52.6565 8 61.6 8L100 8L154.793 62.7933L154.793 62.7934C156.454 64.4543 157.285 65.2848 157.923 66.2239C158.845 67.5811 159.479 69.1131 159.785 70.725C159.997 71.8404 159.997 73.0264 159.995 75.3985C159.96 124.317 159.938 124.799 159.937 154.366C159.937 163.332 159.937 167.816 158.532 171.363C156.499 176.498 152.434 180.562 147.299 182.596C143.752 184 139.279 184 130.334 184Z`,fill:`#3186FF`})}),(0,$.jsxs)(`g`,{mask:`url(#google-docs-2026-mask0_37242_8762)`,children:[(0,$.jsx)(`path`,{d:`M159.94 184L31.9999 184L31.9999 8.00001L99.9999 8L159.999 68L159.94 184Z`,fill:`#3186FF`}),(0,$.jsx)(`g`,{filter:`url(#google-docs-2026-filter0_f_37242_8762)`,children:(0,$.jsx)(`path`,{d:`M43 192H149V70.2271V20H43V192Z`,fill:`url(#google-docs-2026-paint0_linear_37242_8762)`})})]}),(0,$.jsx)(`path`,{d:`M154.995 62.9951C152.489 61.1143 149.375 60 146 60H112.8C105.731 60 100 54.2692 100 47.2002V8L154.995 62.9951Z`,fill:`#76BBFF`}),(0,$.jsx)(`rect`,{x:64.001,y:114,width:64,height:12,rx:6,fill:`white`}),(0,$.jsx)(`rect`,{x:64.001,y:143,width:48,height:12,rx:6,fill:`white`}),(0,$.jsxs)(`defs`,{children:[(0,$.jsxs)(`filter`,{id:`google-docs-2026-filter0_f_37242_8762`,x:31,y:8,width:130,height:196,filterUnits:`userSpaceOnUse`,colorInterpolationFilters:`sRGB`,children:[(0,$.jsx)(`feFlood`,{floodOpacity:0,result:`BackgroundImageFix`}),(0,$.jsx)(`feBlend`,{mode:`normal`,in:`SourceGraphic`,in2:`BackgroundImageFix`,result:`shape`}),(0,$.jsx)(`feGaussianBlur`,{stdDeviation:6,result:`effect1_foregroundBlur_37242_8762`})]}),(0,$.jsxs)(`linearGradient`,{id:`google-docs-2026-paint0_linear_37242_8762`,x1:96,y1:59.2839,x2:54.6124,y2:171.338,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{offset:.33,stopColor:`#3186FF`}),(0,$.jsx)(`stop`,{offset:1,stopColor:`#A9A8FF`})]})]})]}),Gt=e=>(0,$.jsxs)(`svg`,{xmlns:`http://www.w3.org/2000/svg`,width:192,height:192,fill:`none`,viewBox:`0 0 192 192`,...e,children:[(0,$.jsx)(`path`,{fill:`#009954`,d:`M8 74.6c0-8.943 0-13.415 1.404-16.962a20 20 0 0 1 11.234-11.233C24.185 45 28.656 45 37.6 45h60.8c8.943 0 13.415 0 16.962 1.404a20 20 0 0 1 11.234 11.234C128 61.185 128 65.656 128 74.6v42.8c0 8.943 0 13.415-1.404 16.962a20 20 0 0 1-11.234 11.234C111.815 147 107.343 147 98.4 147H37.6c-8.943 0-13.415 0-16.963-1.404a20 20 0 0 1-11.233-11.234C8 130.815 8 126.343 8 117.4z`}),(0,$.jsx)(`mask`,{id:`google-sheets-2026-a`,width:160,height:128,x:24,y:32,maskUnits:`userSpaceOnUse`,style:{maskType:`alpha`},children:(0,$.jsx)(`rect`,{width:160,height:128,x:24,y:32,fill:`#0ebc5f`,rx:20})}),(0,$.jsxs)(`g`,{mask:`url(#google-sheets-2026-a)`,children:[(0,$.jsx)(`path`,{fill:`#0ebc5f`,d:`M24 32h160v128H24z`}),(0,$.jsx)(`g`,{filter:`url(#google-sheets-2026-b)`,children:(0,$.jsx)(`rect`,{width:144,height:102,fill:`url(#google-sheets-2026-c)`,rx:25.6,transform:`matrix(1 0 0 -1 8 147)`})})]}),(0,$.jsx)(`path`,{stroke:`#fff`,strokeLinecap:`round`,strokeWidth:12,d:`M80 121h84m-20 19V76`}),(0,$.jsxs)(`defs`,{children:[(0,$.jsxs)(`linearGradient`,{id:`google-sheets-2026-c`,x1:122.24,x2:20.76,y1:43.31,y2:43.31,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{stopColor:`#0ebc5f`}),(0,$.jsx)(`stop`,{offset:.95,stopColor:`#78c9ff`})]}),(0,$.jsxs)(`filter`,{id:`google-sheets-2026-b`,width:168,height:126,x:-4,y:33,colorInterpolationFilters:`sRGB`,filterUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`feFlood`,{floodOpacity:0,result:`BackgroundImageFix`}),(0,$.jsx)(`feBlend`,{in:`SourceGraphic`,in2:`BackgroundImageFix`,result:`shape`}),(0,$.jsx)(`feGaussianBlur`,{result:`effect1_foregroundBlur_37435_8174`,stdDeviation:6})]})]})]}),Kt=e=>(0,$.jsxs)(`svg`,{xmlns:`http://www.w3.org/2000/svg`,width:192,height:192,fill:`none`,viewBox:`0 0 192 192`,...e,children:[(0,$.jsx)(`path`,{fill:`url(#google-slides-2026-a)`,d:`M12.591 63.318c-2.493-15.262 7.858-29.655 23.12-32.148l96.724-15.8c15.262-2.492 29.655 7.859 32.148 23.12l14.732 90.189c2.493 15.262-7.858 29.655-23.12 32.148l-96.724 15.8c-15.262 2.493-29.655-7.858-32.148-23.12z`}),(0,$.jsx)(`path`,{fill:`url(#google-slides-2026-b)`,d:`M12 61.6c0-8.943 0-13.415 1.405-16.962a20 20 0 0 1 11.233-11.233C28.185 32 32.656 32 41.6 32h108.8c8.943 0 13.415 0 16.962 1.404a20 20 0 0 1 11.234 11.234C180 48.185 180 52.657 180 61.6v68.8c0 8.943 0 13.415-1.404 16.962a20 20 0 0 1-11.234 11.234C163.815 160 159.343 160 150.4 160H41.6c-8.943 0-13.415 0-16.963-1.404a20 20 0 0 1-11.232-11.234C12 143.815 12 139.343 12 130.4z`}),(0,$.jsx)(`mask`,{id:`google-slides-2026-e`,width:168,height:128,x:12,y:32,maskUnits:`userSpaceOnUse`,style:{maskType:`alpha`},children:(0,$.jsx)(`path`,{fill:`#fec700`,d:`M12 61.6c0-8.943 0-13.415 1.405-16.962a20 20 0 0 1 11.233-11.233C28.185 32 32.656 32 41.6 32h108.8c8.943 0 13.415 0 16.962 1.404a20 20 0 0 1 11.234 11.234C180 48.185 180 52.657 180 61.6v68.8c0 8.943 0 13.415-1.404 16.962a20 20 0 0 1-11.234 11.234C163.815 160 159.343 160 150.4 160H41.6c-8.943 0-13.415 0-16.963-1.404a20 20 0 0 1-11.232-11.234C12 143.815 12 139.343 12 130.4z`})}),(0,$.jsxs)(`g`,{filter:`url(#google-slides-2026-c)`,mask:`url(#google-slides-2026-e)`,children:[(0,$.jsx)(`path`,{fill:`#ffbe00`,d:`m33.74 191.516 144.396-21.58L153.304 3.781 8.907 25.361z`}),(0,$.jsx)(`path`,{fill:`url(#google-slides-2026-f)`,d:`m33.74 191.516 144.396-21.58L153.304 3.781 8.907 25.361z`})]}),(0,$.jsx)(`path`,{fill:`#fff`,fillRule:`evenodd`,d:`M148 58a6 6 0 0 1 6 6v64a6 6 0 0 1-6 6H44l-.309-.008A6 6 0 0 1 38 128V64a6 6 0 0 1 5.691-5.992L44 58zm-98 64h92V70H50z`,clipRule:`evenodd`}),(0,$.jsxs)(`defs`,{children:[(0,$.jsxs)(`linearGradient`,{id:`google-slides-2026-a`,x1:84.07,x2:157.2,y1:23.27,y2:160.82,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{offset:.2,stopColor:`#ffdb0f`}),(0,$.jsx)(`stop`,{offset:.67,stopColor:`#ffbe00`}),(0,$.jsx)(`stop`,{offset:.91,stopColor:`#ffa8e3`})]}),(0,$.jsxs)(`linearGradient`,{id:`google-slides-2026-b`,x1:96,x2:96,y1:32,y2:160,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{stopColor:`#ffbe00`}),(0,$.jsx)(`stop`,{offset:1,stopColor:`#fec700`})]}),(0,$.jsxs)(`linearGradient`,{id:`google-slides-2026-f`,x1:108.52,x2:83.96,y1:168.16,y2:25.27,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{offset:.07,stopColor:`#fff549`}),(0,$.jsx)(`stop`,{offset:.78,stopColor:`#ffbe00`,stopOpacity:0})]}),(0,$.jsxs)(`filter`,{id:`google-slides-2026-c`,width:193.23,height:211.73,x:-3.09,y:-8.22,colorInterpolationFilters:`sRGB`,filterUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`feFlood`,{floodOpacity:0,result:`BackgroundImageFix`}),(0,$.jsx)(`feBlend`,{in:`SourceGraphic`,in2:`BackgroundImageFix`,result:`shape`}),(0,$.jsx)(`feGaussianBlur`,{result:`effect1_foregroundBlur_37552_9023`,stdDeviation:6})]})]})]}),qt=e=>(0,$.jsxs)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`path`,{d:`M3.6001 3.6C3.6001 2.71634 4.31644 2 5.2001 2H11.6001L16.4001 6.8V16.4C16.4001 17.2837 15.6838 18 14.8001 18H5.2001C4.31644 18 3.6001 17.2837 3.6001 16.4V3.6Z`,fill:`#FF4343`}),(0,$.jsx)(`path`,{d:`M13.2773 14.0014H12.2061V10.5521H14.8002V11.4649H13.2773V12.0125H14.5264V12.9013H13.2773V14.0014Z`,fill:`white`}),(0,$.jsx)(`path`,{d:`M8.60938 14.0014V10.5521H9.94489C11.0834 10.5521 11.7512 11.2247 11.7512 12.2768C11.7512 13.3288 11.0834 14.0014 9.94489 14.0014H8.60938ZM9.67586 13.0886H9.94489C10.4301 13.0886 10.6703 12.7668 10.6703 12.2768C10.6703 11.7868 10.4301 11.4649 9.94489 11.4649H9.67586V13.0886Z`,fill:`white`}),(0,$.jsx)(`path`,{d:`M6.2907 13.0118V14.0014H5.2002V10.5521H6.7711C7.65504 10.5521 8.19789 10.9749 8.19789 11.7819C8.19789 12.589 7.65504 13.0118 6.7711 13.0118H6.2907ZM6.2907 12.1086H6.71346C6.99689 12.1086 7.1218 11.9789 7.1218 11.7819C7.1218 11.585 6.99689 11.4553 6.71346 11.4553H6.2907V12.1086Z`,fill:`white`}),(0,$.jsx)(`path`,{d:`M11.6001 2L16.4001 6.8H13.2001C12.3164 6.8 11.6001 6.08366 11.6001 5.2V2Z`,fill:`#A53636`})]}),Jt=e=>(0,$.jsxs)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`path`,{d:`M3.6001 3.6C3.6001 2.71634 4.31644 2 5.2001 2H11.6001L16.4001 6.8V16.4C16.4001 17.2837 15.6838 18 14.8001 18H5.2001C4.31644 18 3.6001 17.2837 3.6001 16.4V3.6Z`,fill:`#F4B400`}),(0,$.jsx)(`path`,{d:`M11.6001 2L16.4001 6.8H13.2001C12.3164 6.8 11.6001 6.08366 11.6001 5.2V2Z`,fill:`#9D7607`}),(0,$.jsx)(`path`,{fillRule:`evenodd`,clipRule:`evenodd`,d:`M6.00049 9.39999C6.00049 9.06862 6.26912 8.79999 6.60049 8.79999H13.4005C13.7319 8.79999 14.0005 9.06862 14.0005 9.39999V14.2C14.0005 14.5314 13.7319 14.8 13.4005 14.8H6.60049C6.26912 14.8 6.00049 14.5314 6.00049 14.2V9.39999ZM7.20049 10.4V13.2H12.8005V10.4H7.20049Z`,fill:`white`})]}),Yt=e=>(0,$.jsxs)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`path`,{d:`M3.6001 3.6C3.6001 2.71634 4.31644 2 5.2001 2H11.6001L16.4001 6.8V16.4C16.4001 17.2837 15.6838 18 14.8001 18H5.2001C4.31644 18 3.6001 17.2837 3.6001 16.4V3.6Z`,fill:`#34A853`}),(0,$.jsx)(`path`,{d:`M11.6001 2L16.4001 6.8H13.2001C12.3164 6.8 11.6001 6.08366 11.6001 5.2V2Z`,fill:`#188038`}),(0,$.jsx)(`path`,{d:`M6.40049 8.79999C6.17957 8.79999 6.00049 8.97907 6.00049 9.19999V14.4C6.00049 14.6209 6.17957 14.8 6.40049 14.8H13.6005C13.8214 14.8 14.0005 14.6209 14.0005 14.4V9.19999C14.0005 8.97907 13.8214 8.79999 13.6005 8.79999H6.40049ZM9.40049 13.6H7.20049V12.2L9.40049 12.2V13.6ZM9.40049 11.4H7.20049V9.99999H9.40049V11.4ZM12.8005 13.6H10.6005V12.2L12.8005 12.2V13.6ZM12.8005 11.4H10.6005V9.99999H12.8005V11.4Z`,fill:`white`})]}),Xt=e=>(0,$.jsxs)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`path`,{d:`M3.6001 3.6C3.6001 2.71634 4.31644 2 5.2001 2H11.6001L16.4001 6.8V16.4C16.4001 17.2837 15.6838 18 14.8001 18H5.2001C4.31644 18 3.6001 17.2837 3.6001 16.4V3.6Z`,fill:`#4285F4`}),(0,$.jsx)(`path`,{d:`M11.6001 2L16.4001 6.8H13.2001C12.3164 6.8 11.6001 6.08366 11.6001 5.2V2Z`,fill:`#1967D2`}),(0,$.jsx)(`path`,{d:`M6.80029 11.4C6.80029 11.2895 6.88984 11.2 7.00029 11.2H13.0003C13.1108 11.2 13.2003 11.2895 13.2003 11.4V12.2C13.2003 12.3104 13.1108 12.4 13.0003 12.4H7.00029C6.88984 12.4 6.80029 12.3104 6.80029 12.2V11.4ZM6.80029 13.8C6.80029 13.6895 6.88984 13.6 7.00029 13.6H11.4003C11.5108 13.6 11.6003 13.6895 11.6003 13.8V14.6C11.6003 14.7104 11.5107 14.8 11.4003 14.8H7.00029C6.88984 14.8 6.80029 14.7104 6.80029 14.6V13.8Z`,fill:`white`}),(0,$.jsx)(`path`,{d:`M6.80029 8.99999C6.80029 8.88953 6.88984 8.79999 7.00029 8.79999H13.0003C13.1108 8.79999 13.2003 8.88953 13.2003 8.99999V9.79999C13.2003 9.91044 13.1108 9.99999 13.0003 9.99999H7.00029C6.88984 9.99999 6.80029 9.91044 6.80029 9.79999V8.99999Z`,fill:`white`})]}),Zt=e=>(0,$.jsx)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:(0,$.jsx)(`path`,{d:`M9.99996 2.08002C14.373 2.08002 17.915 5.62198 17.915 9.99502C17.9145 11.6534 17.3941 13.2699 16.4268 14.617C15.4595 15.9641 14.0941 16.9739 12.5229 17.5044C12.1271 17.5835 11.9787 17.3362 11.9787 17.1284C11.9787 16.8613 11.9886 16.0104 11.9886 14.9518C11.9886 14.2098 11.7413 13.7349 11.4543 13.4875C13.2154 13.2896 15.0656 12.6169 15.0656 9.57948C15.0656 8.70883 14.7589 8.00637 14.2543 7.45232C14.3334 7.25445 14.6104 6.44316 14.1751 5.35485C14.1751 5.35485 13.5122 5.13719 11.9985 6.16614C11.3653 5.98805 10.6925 5.899 10.0197 5.899C9.34697 5.899 8.6742 5.98805 8.041 6.16614C6.52726 5.14708 5.86437 5.35485 5.86437 5.35485C5.42905 6.44316 5.70607 7.25445 5.78522 7.45232C5.28064 8.00637 4.97394 8.71872 4.97394 9.57948C4.97394 12.607 6.81417 13.2896 8.57526 13.4875C8.3477 13.6854 8.13994 14.0317 8.07068 14.5461C7.61557 14.7539 6.47779 15.0903 5.76544 13.8932C5.61703 13.6557 5.17181 13.072 4.54851 13.0819C3.88562 13.0918 4.28137 13.4578 4.5584 13.6062C4.89479 13.7942 5.28064 14.4967 5.36969 14.7242C5.52799 15.1694 6.04246 16.0203 8.03111 15.6542C8.03111 16.3171 8.041 16.9404 8.041 17.1284C8.041 17.3362 7.89259 17.5736 7.49684 17.5044C5.92041 16.9796 4.54923 15.9718 3.57782 14.6239C2.60641 13.276 2.08409 11.6565 2.08496 9.99502C2.08496 5.62198 5.62692 2.08002 9.99996 2.08002Z`,fill:`currentColor`})}),Qt=e=>(0,$.jsxs)(`svg`,{xmlns:`http://www.w3.org/2000/svg`,width:192,height:192,fill:`none`,viewBox:`0 0 192 192`,...e,children:[(0,$.jsx)(`path`,{fill:`url(#gmail-2026-a)`,d:`M146 44h38v110c0 6.627-5.373 12-12 12h-20a6 6 0 0 1-6-6z`}),(0,$.jsx)(`path`,{fill:`#fc413d`,d:`M46 44H8v110c0 6.627 5.373 12 12 12h20a6 6 0 0 0 6-6z`}),(0,$.jsx)(`path`,{fill:`url(#gmail-2026-b)`,d:`M39.226 30.456c-8.033-6.752-20.018-5.714-26.77 2.319-6.752 8.032-5.714 20.017 2.319 26.77l76.078 63.949a8 8 0 0 0 10.295 0l76.078-63.95c8.032-6.752 9.07-18.737 2.318-26.77-6.752-8.032-18.737-9.07-26.769-2.318L96 78.18z`}),(0,$.jsxs)(`defs`,{children:[(0,$.jsxs)(`linearGradient`,{id:`gmail-2026-a`,x1:165,x2:165,y1:44,y2:166,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{stopColor:`#60d673`}),(0,$.jsx)(`stop`,{offset:.17,stopColor:`#42c868`}),(0,$.jsx)(`stop`,{offset:.39,stopColor:`#0ebc5f`}),(0,$.jsx)(`stop`,{offset:.62,stopColor:`#00a9bb`}),(0,$.jsx)(`stop`,{offset:.86,stopColor:`#3c90ff`}),(0,$.jsx)(`stop`,{offset:1,stopColor:`#3186ff`})]}),(0,$.jsxs)(`linearGradient`,{id:`gmail-2026-b`,x1:8,x2:184,y1:46.13,y2:46.13,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{offset:.08,stopColor:`#ff63a0`}),(0,$.jsx)(`stop`,{offset:.3,stopColor:`#fc413d`}),(0,$.jsx)(`stop`,{offset:.5,stopColor:`#fc413d`}),(0,$.jsx)(`stop`,{offset:.65,stopColor:`#fc413d`}),(0,$.jsx)(`stop`,{offset:.72,stopColor:`#fc5c30`}),(0,$.jsx)(`stop`,{offset:.86,stopColor:`#feb10c`}),(0,$.jsx)(`stop`,{offset:.91,stopColor:`#fec700`}),(0,$.jsx)(`stop`,{offset:.96,stopColor:`#ffdb0f`})]})]})]}),$t=e=>(0,$.jsxs)(`svg`,{xmlns:`http://www.w3.org/2000/svg`,width:192,height:192,viewBox:`0 0 192 192`,fill:`none`,...e,children:[(0,$.jsx)(`path`,{fill:`#bbe2ff`,d:`M32 36.8C32 20.894 44.894 8 60.8 8h70.4C147.106 8 160 20.894 160 36.8v30.4c0 15.906-12.894 28.8-28.8 28.8H60.8C44.894 96 32 83.106 32 67.2z`}),(0,$.jsx)(`path`,{fill:`#3c90ff`,d:`M19.867 49.392C17.818 33.82 29.94 20 45.645 20h100.71c15.706 0 27.827 13.82 25.778 29.392L166 96l6.133 46.608C174.182 158.18 162.061 172 146.355 172H45.645c-15.706 0-27.827-13.82-25.778-29.392L26 96z`}),(0,$.jsx)(`mask`,{id:`google-calendar-2026-a`,width:154,height:152,x:19,y:20,maskUnits:`userSpaceOnUse`,style:{maskType:`alpha`},children:(0,$.jsx)(`path`,{fill:`#3c90ff`,d:`M19.867 49.392C17.818 33.82 29.94 20 45.645 20h100.71c15.706 0 27.827 13.82 25.778 29.392L166 96l6.133 46.608C174.182 158.18 162.061 172 146.355 172H45.645c-15.706 0-27.827-13.82-25.778-29.392L26 96z`})}),(0,$.jsx)(`g`,{mask:`url(#google-calendar-2026-a)`,children:(0,$.jsx)(`path`,{fill:`url(#google-calendar-2026-b)`,d:`M0 0h166v76H0z`,transform:`matrix(1 0 0 -1 13 172)`})}),(0,$.jsx)(`mask`,{id:`google-calendar-2026-c`,width:154,height:152,x:19,y:20,maskUnits:`userSpaceOnUse`,style:{maskType:`alpha`},children:(0,$.jsx)(`path`,{fill:`#3186ff`,d:`M19.867 49.392C17.818 33.82 29.94 20 45.645 20h100.71c15.706 0 27.827 13.82 25.778 29.392L166 96l6.133 46.608C174.182 158.18 162.061 172 146.355 172H45.645c-15.706 0-27.827-13.82-25.778-29.392L26 96z`})}),(0,$.jsx)(`g`,{mask:`url(#google-calendar-2026-c)`,children:(0,$.jsx)(`path`,{fill:`url(#google-calendar-2026-d)`,d:`M32 27.2C32 16.596 40.596 8 51.2 8h89.6c10.604 0 19.2 8.596 19.2 19.2V96H32z`,filter:`url(#google-calendar-2026-e)`})}),(0,$.jsx)(`path`,{fill:`#fff`,d:`M75.353 133.336q-6.282 0-10.777-2.043t-7.61-5.465q-3.065-3.474-4.342-6.793T51.603 115a2.07 2.07 0 0 1 1.021-1.124l5.67-2.247q.714-.357 1.43-.102.714.204 1.685 2.349 1.022 2.145 2.86 4.546a14.3 14.3 0 0 0 4.495 3.728q2.606 1.328 6.435 1.328 6.18 0 9.807-3.575 3.677-3.575 3.677-9.091 0-5.976-3.882-9.194-3.881-3.269-10.266-3.269h-5.362a1.9 1.9 0 0 1-1.328-.51q-.51-.562-.511-1.277v-5.465q0-.767.51-1.277a1.82 1.82 0 0 1 1.329-.562h4.647q5.721 0 9.194-3.116t3.473-8.07q0-4.902-3.116-7.916t-8.58-3.014q-3.065 0-5.312 1.022a11.5 11.5 0 0 0-3.882 2.86 22.7 22.7 0 0 0-2.809 3.78q-1.174 1.941-1.89 2.145-.714.153-1.379-.255l-5.363-2.605q-.664-.358-.868-1.124t1.226-3.575q1.481-2.86 4.494-5.823a21 21 0 0 1 7.049-4.597q4.035-1.635 9.398-1.634 9.96 0 15.782 5.26 5.823 5.21 5.823 13.791 0 5.925-2.86 10.266-2.81 4.34-7.968 6.13v.204q6.231 1.838 9.806 6.741 3.627 4.853 3.626 11.594 0 9.654-6.742 15.834-6.74 6.18-17.57 6.18zm51.25-1.175q-.868 0-1.533-.664a2.25 2.25 0 0 1-.612-1.583V73.118l-11.492 8.274q-.614.46-1.431.307a1.96 1.96 0 0 1-1.225-.766l-3.32-4.7a1.98 1.98 0 0 1-.358-1.43q.153-.816.817-1.276l20.379-14.557q.256-.204.562-.306.307-.153.715-.153h4.291q.868 0 1.379.613.562.56.562 1.43v69.36q0 .92-.664 1.583a2 2 0 0 1-1.533.664z`}),(0,$.jsxs)(`defs`,{children:[(0,$.jsxs)(`linearGradient`,{id:`google-calendar-2026-b`,x1:83,x2:83,y1:76,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{stopColor:`#4fa0ff`}),(0,$.jsx)(`stop`,{offset:1,stopColor:`#3186ff`})]}),(0,$.jsxs)(`linearGradient`,{id:`google-calendar-2026-d`,x1:89.06,x2:89.06,y1:21.75,y2:96.39,gradientUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`stop`,{stopColor:`#a9a8ff`}),(0,$.jsx)(`stop`,{offset:.8,stopColor:`#3c90ff`})]}),(0,$.jsxs)(`filter`,{id:`google-calendar-2026-e`,width:152,height:112,x:20,y:-4,colorInterpolationFilters:`sRGB`,filterUnits:`userSpaceOnUse`,children:[(0,$.jsx)(`feFlood`,{floodOpacity:0,result:`BackgroundImageFix`}),(0,$.jsx)(`feBlend`,{in:`SourceGraphic`,in2:`BackgroundImageFix`,result:`shape`}),(0,$.jsx)(`feGaussianBlur`,{result:`effect1_foregroundBlur_37330_7673`,stdDeviation:6})]})]})]}),en=e=>(0,$.jsx)(`svg`,{xmlns:`http://www.w3.org/2000/svg`,viewBox:`0 0 24 24`,...e,children:(0,$.jsx)(`path`,{fill:`#00A1E0`,d:`M10.006 5.415a4.195 4.195 0 0 1 3.045-1.306c1.56 0 2.954.9 3.69 2.205.63-.3 1.35-.45 2.1-.45 2.85 0 5.159 2.34 5.159 5.22s-2.31 5.22-5.176 5.22c-.345 0-.69-.044-1.02-.104a3.75 3.75 0 0 1-3.3 1.95c-.6 0-1.155-.15-1.65-.375A4.314 4.314 0 0 1 8.88 20.4a4.302 4.302 0 0 1-4.05-2.82c-.27.062-.54.076-.825.076-2.204 0-4.005-1.8-4.005-4.05 0-1.5.811-2.805 2.01-3.51-.255-.57-.39-1.2-.39-1.846 0-2.58 2.1-4.65 4.65-4.65 1.53 0 2.85.705 3.72 1.8Z`})}),tn=e=>(0,$.jsxs)(`svg`,{width:20,height:20,viewBox:`0 0 20 20`,fill:`none`,xmlns:`http://www.w3.org/2000/svg`,...e,children:[(0,$.jsx)(`path`,{d:`M5.37305 12.1146C5.37305 13.0446 4.62206 13.7962 3.69287 13.7962C2.76368 13.7962 2.0127 13.0446 2.0127 12.1146C2.0127 11.1847 2.76368 10.4331 3.69287 10.4331H5.37305V12.1146ZM6.21314 12.1146C6.21314 11.1847 6.96413 10.4331 7.89331 10.4331C8.8225 10.4331 9.57349 11.1847 9.57349 12.1146V16.3185C9.57349 17.2484 8.8225 18 7.89331 18C6.96413 18 6.21314 17.2484 6.21314 16.3185V12.1146Z`,fill:`#E01E5A`}),(0,$.jsx)(`path`,{d:`M7.89335 5.36307C6.96416 5.36307 6.21317 4.61147 6.21317 3.68153C6.21317 2.75159 6.96416 2 7.89335 2C8.82254 2 9.57352 2.75159 9.57352 3.68153V5.36307H7.89335ZM7.89335 6.21657C8.82254 6.21657 9.57352 6.96817 9.57352 7.89811C9.57352 8.82805 8.82254 9.57964 7.89335 9.57964H3.68018C2.75099 9.57964 2 8.82805 2 7.89811C2 6.96817 2.75099 6.21657 3.68018 6.21657H7.89335Z`,fill:`#36C5F0`}),(0,$.jsx)(`path`,{d:`M14.6267 7.89811C14.6267 6.96817 15.3777 6.21657 16.3069 6.21657C17.2361 6.21657 17.9871 6.96817 17.9871 7.89811C17.9871 8.82805 17.2361 9.57964 16.3069 9.57964H14.6267V7.89811ZM13.7866 7.89811C13.7866 8.82805 13.0356 9.57964 12.1064 9.57964C11.1773 9.57964 10.4263 8.82805 10.4263 7.89811V3.68153C10.4263 2.75159 11.1773 2 12.1064 2C13.0356 2 13.7866 2.75159 13.7866 3.68153V7.89811Z`,fill:`#2EB67D`}),(0,$.jsx)(`path`,{d:`M12.1064 14.6369C13.0356 14.6369 13.7866 15.3885 13.7866 16.3185C13.7866 17.2484 13.0356 18 12.1064 18C11.1773 18 10.4263 17.2484 10.4263 16.3185V14.6369H12.1064ZM12.1064 13.7962C11.1773 13.7962 10.4263 13.0446 10.4263 12.1146C10.4263 11.1847 11.1773 10.4331 12.1064 10.4331H16.3196C17.2488 10.4331 17.9998 11.1847 17.9998 12.1146C17.9998 13.0446 17.2488 13.7962 16.3196 13.7962H12.1064Z`,fill:`#ECB22E`})]}),nn={figma:Ht,"file-csv":Ut,"file-pdf":qt,"file-presentation":Jt,"file-spreadsheet":Yt,"file-word-document":Xt,git:de,gmail:Qt,"google-calendar":$t,"google-docs":Wt,"google-drive":fe,"google-sheets":Gt,"google-slides":Kt,github:Zt,linear:K,notion:q,salesforce:en,sites:G,slack:tn};function rn(e){return sn(on(e))}function an(e){let t=[e.name,e.id,...e.pluginDisplayNames].map(on);for(let e of t){let t=sn(e);if(t!=null)return t}return null}function on(e){return e.trim().toLowerCase().split(/[^a-z0-9]+/g).filter(e=>e.length>0).join(`-`)}function sn(e){let t=e.replace(/^connector-/u,``);return nn[e]??nn[t]??nn[t.replace(/-mcp-server$/u,``)]??null}var cn=`connectors://`,ln=`light`,un=`dark`;function dn(e){let t=e?.trim();if(t==null||t.length===0||!t.startsWith(cn))return null;let n=t.slice(13),r=((n.split(/[?#]/u)[0]??``).split(`/`)[0]??``).trim();if(r.length===0)return null;let i=n.split(`?`)[1]?.split(`#`)[0]??``;return{connectorId:r,theme:new URLSearchParams(i).get(`theme`)?.toLowerCase()===un?un:ln}}function fn({connectorId:e,theme:t}){return`${e}:${t}`}async function pn({connectorId:e,theme:t}){let n=await v.getInstance().get(`/aip/connectors/${encodeURIComponent(e)}/logo?theme=${t}`,w());return n.body.contentType.toLowerCase().startsWith(`text/plain`)?M(n.body.base64).trim():`data:${n.body.contentType};base64,${n.body.base64}`}var mn=`OAI-Product-Sku`,hn=`CODEX`,gn=`codex`,_n=[`mcp-settings`,`app-connect`];function vn(e,{includeActions:t=!1,includeLogo:n=!1}={}){return{queryKey:[..._n,e,t,n],staleTime:D.FIVE_MINUTES,queryFn:async()=>y.safeGet(`/aip/connectors/{connector_id}`,{parameters:{path:{connector_id:e},query:{include_logo:n,include_actions:t}},additionalHeaders:{[mn]:hn}})}}function yn(e){let t=e.installUrl?.trim();if(!t)return null;let n=new URL(t);return n.hash=Pn(e.id),n.toString()}async function bn({app:e,callbackMode:t=`native`,connector:n,openInBrowser:r,queryClient:i}){let a=n;if(a==null)try{a=await i.fetchQuery(vn(e.id))}catch(t){return P.error(`Failed to resolve app connect flow`,{safe:{appId:e.id},sensitive:{error:t}}),Fn({app:e,openInBrowser:r})?{kind:`browser-fallback`}:{kind:`failed`}}if(a==null)return{kind:`failed`};let o=On(a);if(Dn(a)||o===`UNSUPPORTED`)return Fn({app:e,openInBrowser:r})?{kind:`browser-fallback`}:{kind:`failed`};try{if(o===`NONE`)return await y.safePost(`/aip/connectors/links/noauth`,{requestBody:{connector_id:a.id,name:a.name,action_names:[]},additionalHeaders:{[mn]:hn}}),{kind:`connected-directly`};let n=t===`browser`?An(e):await kn(),i=(await y.safePost(`/aip/connectors/links/oauth`,{requestBody:{connector_id:a.id,name:a.name,action_names:null,callback_url:n,post_auth_url:jn(e)},additionalHeaders:{[mn]:hn}})).redirect_url?.trim();if(!i)throw Error(`OAuth redirect URL missing in connector response.`);return r(i),{kind:`oauth-started`,redirectUrl:i}}catch(t){return P.error(`Failed to connect app {}`,{safe:{templateArgs:[e.id]},sensitive:{error:t}}),Fn({app:e,openInBrowser:r})?{kind:`browser-fallback`}:{kind:`failed`}}}async function xn({app:e,authReason:t,fallbackUrl:n,linkId:r,openInBrowser:i,queryClient:a,requestedScopes:o}){if(t===`missing_link`)return bn({app:e,openInBrowser:i,queryClient:a});let s=r?.trim();if(s)try{let t=(await y.safePost(`/aip/connectors/links/oauth/reauth`,{requestBody:{callback_url:await kn(),link_id:s,post_auth_url:jn(e),requested_scopes:o},additionalHeaders:{[mn]:hn}})).redirect_url?.trim();if(!t)throw Error(`OAuth redirect URL missing in connector response.`);return i(t),{kind:`oauth-started`,redirectUrl:t}}catch(t){P.error(`Failed to reauthenticate app {}`,{safe:{templateArgs:[e.id]},sensitive:{error:t}})}let c=n.trim();return c?(i(c),{kind:`browser-fallback`}):{kind:`failed`}}function Sn({intl:e}){return e.formatMessage({id:`settings.mcp.appConnectModal.oauthStartedElectron`,defaultMessage:`Finish connecting in your browser.`,description:`Toast shown after starting OAuth from MCP settings app connect modal`})}function Cn({appName:e,intl:t}){return t.formatMessage({id:`settings.mcp.appConnectModal.connected`,defaultMessage:`{appName} is now connected.`,description:`Toast shown when a no-auth app is connected directly from MCP settings`},{appName:e})}function wn(e){return e.formatMessage({id:`settings.mcp.appConnectModal.connectFailed`,defaultMessage:`Failed to connect app.`,description:`Toast shown when starting an app connection fails`})}function Tn(e){return e.formatMessage({id:`settings.mcp.appConnectModal.installUrlMissing`,defaultMessage:`This app does not provide a browser setup URL right now.`,description:`Toast shown when app connect fallback is attempted but no install URL is available`})}function En(e){if(typeof e!=`object`||!e)return!1;let t=e,n=t.properties;if(n&&typeof n==`object`)return Object.keys(n).length>0;let r=t.required;return!!(Array.isArray(r)&&r.length>0)}function Dn(e){return En(e.link_params_schema)}function On(e){return e.supported_auth.some(e=>e.type===`OAUTH`)?`OAUTH`:e.supported_auth.some(e=>e.type===`NONE`)?`NONE`:`UNSUPPORTED`}async function kn(){let{callbackUrl:e}=await x(`app-connect-oauth-callback-url`);return e}function An(e){return Nn(e)+`/connector_platform_oauth_redirect`}function jn(e){let t=yn(e);if(t!=null)return t;let n=new URL(`/gpts/editor`,Nn(e));return n.hash=Pn(e.id),n.toString()}function Mn(e){let t=e.installUrl?.trim();if(!t)return null;let n=new URL(t);return n.hash=Pn(e.id,{addConnectorLink:!0}),n.toString()}function Nn(e){let t=e.installUrl?.trim();return t?new URL(t).origin:`https://chatgpt.com`}function Pn(e,{addConnectorLink:t=!1}={}){let n=new URLSearchParams([[`connector`,e]]);return t&&n.set(`add-connector-link`,`true`),n.set(`product-sku`,hn),n.set(`referrer`,gn),`settings/Connectors?${n.toString()}`}function Fn({app:e,openInBrowser:t}){let n=Mn(e);return n==null?!1:(t(n),!0)}var In=t(((e,t)=>{function n(e,t,n,r){var i=-1,a=e==null?0:e.length;for(r&&a&&(n=e[++i]);++i<a;)n=t(n,e[i],i,e);return n}t.exports=n})),Ln=t(((e,t)=>{function n(e){return function(t){return e?.[t]}}t.exports=n})),Rn=t(((e,t)=>{t.exports=Ln()({À:`A`,Á:`A`,Â:`A`,Ã:`A`,Ä:`A`,Å:`A`,à:`a`,á:`a`,â:`a`,ã:`a`,ä:`a`,å:`a`,Ç:`C`,ç:`c`,Ð:`D`,ð:`d`,È:`E`,É:`E`,Ê:`E`,Ë:`E`,è:`e`,é:`e`,ê:`e`,ë:`e`,Ì:`I`,Í:`I`,Î:`I`,Ï:`I`,ì:`i`,í:`i`,î:`i`,ï:`i`,Ñ:`N`,ñ:`n`,Ò:`O`,Ó:`O`,Ô:`O`,Õ:`O`,Ö:`O`,Ø:`O`,ò:`o`,ó:`o`,ô:`o`,õ:`o`,ö:`o`,ø:`o`,Ù:`U`,Ú:`U`,Û:`U`,Ü:`U`,ù:`u`,ú:`u`,û:`u`,ü:`u`,Ý:`Y`,ý:`y`,ÿ:`y`,Æ:`Ae`,æ:`ae`,Þ:`Th`,þ:`th`,ß:`ss`,Ā:`A`,Ă:`A`,Ą:`A`,ā:`a`,ă:`a`,ą:`a`,Ć:`C`,Ĉ:`C`,Ċ:`C`,Č:`C`,ć:`c`,ĉ:`c`,ċ:`c`,č:`c`,Ď:`D`,Đ:`D`,ď:`d`,đ:`d`,Ē:`E`,Ĕ:`E`,Ė:`E`,Ę:`E`,Ě:`E`,ē:`e`,ĕ:`e`,ė:`e`,ę:`e`,ě:`e`,Ĝ:`G`,Ğ:`G`,Ġ:`G`,Ģ:`G`,ĝ:`g`,ğ:`g`,ġ:`g`,ģ:`g`,Ĥ:`H`,Ħ:`H`,ĥ:`h`,ħ:`h`,Ĩ:`I`,Ī:`I`,Ĭ:`I`,Į:`I`,İ:`I`,ĩ:`i`,ī:`i`,ĭ:`i`,į:`i`,ı:`i`,Ĵ:`J`,ĵ:`j`,Ķ:`K`,ķ:`k`,ĸ:`k`,Ĺ:`L`,Ļ:`L`,Ľ:`L`,Ŀ:`L`,Ł:`L`,ĺ:`l`,ļ:`l`,ľ:`l`,ŀ:`l`,ł:`l`,Ń:`N`,Ņ:`N`,Ň:`N`,Ŋ:`N`,ń:`n`,ņ:`n`,ň:`n`,ŋ:`n`,Ō:`O`,Ŏ:`O`,Ő:`O`,ō:`o`,ŏ:`o`,ő:`o`,Ŕ:`R`,Ŗ:`R`,Ř:`R`,ŕ:`r`,ŗ:`r`,ř:`r`,Ś:`S`,Ŝ:`S`,Ş:`S`,Š:`S`,ś:`s`,ŝ:`s`,ş:`s`,š:`s`,Ţ:`T`,Ť:`T`,Ŧ:`T`,ţ:`t`,ť:`t`,ŧ:`t`,Ũ:`U`,Ū:`U`,Ŭ:`U`,Ů:`U`,Ű:`U`,Ų:`U`,ũ:`u`,ū:`u`,ŭ:`u`,ů:`u`,ű:`u`,ų:`u`,Ŵ:`W`,ŵ:`w`,Ŷ:`Y`,ŷ:`y`,Ÿ:`Y`,Ź:`Z`,Ż:`Z`,Ž:`Z`,ź:`z`,ż:`z`,ž:`z`,Ĳ:`IJ`,ĳ:`ij`,Œ:`Oe`,œ:`oe`,ŉ:`'n`,ſ:`s`})})),zn=t(((e,t)=>{var n=Rn(),r=R(),i=/[\xc0-\xd6\xd8-\xf6\xf8-\xff\u0100-\u017f]/g,a=RegExp(`[\\u0300-\\u036f\\ufe20-\\ufe2f\\u20d0-\\u20ff]`,`g`);function o(e){return e=r(e),e&&e.replace(i,n).replace(a,``)}t.exports=o})),Bn=t(((e,t)=>{var n=/[^\x00-\x2f\x3a-\x40\x5b-\x60\x7b-\x7f]+/g;function r(e){return e.match(n)||[]}t.exports=r})),Vn=t(((e,t)=>{var n=/[a-z][A-Z]|[A-Z]{2}[a-z]|[0-9][a-zA-Z]|[a-zA-Z][0-9]|[^a-zA-Z0-9 ]/;function r(e){return n.test(e)}t.exports=r})),Hn=t(((e,t)=>{var n=`\\ud800-\\udfff`,r=`\\u0300-\\u036f\\ufe20-\\ufe2f\\u20d0-\\u20ff`,i=`\\u2700-\\u27bf`,a=`a-z\\xdf-\\xf6\\xf8-\\xff`,o=`\\xac\\xb1\\xd7\\xf7`,s=`\\x00-\\x2f\\x3a-\\x40\\x5b-\\x60\\x7b-\\xbf`,c=`\\u2000-\\u206f`,l=` \\t\\x0b\\f\\xa0\\ufeff\\n\\r\\u2028\\u2029\\u1680\\u180e\\u2000\\u2001\\u2002\\u2003\\u2004\\u2005\\u2006\\u2007\\u2008\\u2009\\u200a\\u202f\\u205f\\u3000`,u=`A-Z\\xc0-\\xd6\\xd8-\\xde`,d=`\\ufe0e\\ufe0f`,f=o+s+c+l,p=`['’]`,m=`[`+f+`]`,h=`[`+r+`]`,g=`\\d+`,_=`[`+i+`]`,v=`[`+a+`]`,y=`[^`+n+f+g+i+a+u+`]`,b=`(?:`+h+`|\\ud83c[\\udffb-\\udfff])`,x=`[^`+n+`]`,S=`(?:\\ud83c[\\udde6-\\uddff]){2}`,C=`[\\ud800-\\udbff][\\udc00-\\udfff]`,w=`[`+u+`]`,T=`\\u200d`,E=`(?:`+v+`|`+y+`)`,D=`(?:`+w+`|`+y+`)`,O=`(?:`+p+`(?:d|ll|m|re|s|t|ve))?`,k=`(?:`+p+`(?:D|LL|M|RE|S|T|VE))?`,A=b+`?`,j=`[`+d+`]?`,M=`(?:`+T+`(?:`+[x,S,C].join(`|`)+`)`+j+A+`)*`,N=`\\d*(?:1st|2nd|3rd|(?![123])\\dth)(?=\\b|[A-Z_])`,P=`\\d*(?:1ST|2ND|3RD|(?![123])\\dTH)(?=\\b|[a-z_])`,F=j+A+M,I=`(?:`+[_,S,C].join(`|`)+`)`+F,L=RegExp([w+`?`+v+`+`+O+`(?=`+[m,w,`$`].join(`|`)+`)`,D+`+`+k+`(?=`+[m,w+E,`$`].join(`|`)+`)`,w+`?`+E+`+`+O,w+`+`+k,P,N,g,I].join(`|`),`g`);function ee(e){return e.match(L)||[]}t.exports=ee})),Un=t(((e,t)=>{var n=Bn(),r=Vn(),i=R(),a=Hn();function o(e,t,o){return e=i(e),t=o?void 0:t,t===void 0?r(e)?a(e):n(e):e.match(t)||[]}t.exports=o})),Wn=t(((e,t)=>{var n=In(),r=zn(),i=Un(),a=RegExp(`['’]`,`g`);function o(e){return function(t){return n(i(r(t).replace(a,``)),e,``)}}t.exports=o})),Gn=t(((e,t)=>{var n=ce();function r(e,t,r){var i=e.length;return r=r===void 0?i:r,!t&&r>=i?e:n(e,t,r)}t.exports=r})),Kn=t(((e,t)=>{var n=RegExp(`[\\u200d\\ud800-\\udfff\\u0300-\\u036f\\ufe20-\\ufe2f\\u20d0-\\u20ff\\ufe0e\\ufe0f]`);function r(e){return n.test(e)}t.exports=r})),qn=t(((e,t)=>{function n(e){return e.split(``)}t.exports=n})),Jn=t(((e,t)=>{var n=`\\ud800-\\udfff`,r=`\\u0300-\\u036f\\ufe20-\\ufe2f\\u20d0-\\u20ff`,i=`\\ufe0e\\ufe0f`,a=`[`+n+`]`,o=`[`+r+`]`,s=`\\ud83c[\\udffb-\\udfff]`,c=`(?:`+o+`|`+s+`)`,l=`[^`+n+`]`,u=`(?:\\ud83c[\\udde6-\\uddff]){2}`,d=`[\\ud800-\\udbff][\\udc00-\\udfff]`,f=`\\u200d`,p=c+`?`,m=`[`+i+`]?`,h=`(?:`+f+`(?:`+[l,u,d].join(`|`)+`)`+m+p+`)*`,g=m+p+h,_=`(?:`+[l+o+`?`,o,u,d,a].join(`|`)+`)`,v=RegExp(s+`(?=`+s+`)|`+_+g,`g`);function y(e){return e.match(v)||[]}t.exports=y})),Yn=t(((e,t)=>{var n=qn(),r=Kn(),i=Jn();function a(e){return r(e)?i(e):n(e)}t.exports=a})),Xn=t(((e,t)=>{var n=Gn(),r=Kn(),i=Yn(),a=R();function o(e){return function(t){t=a(t);var o=r(t)?i(t):void 0,s=o?o[0]:t.charAt(0),c=o?n(o,1).join(``):t.slice(1);return s[e]()+c}}t.exports=o})),Zn=t(((e,t)=>{t.exports=Xn()(`toUpperCase`)})),Qn=t(((e,t)=>{var n=Wn(),r=Zn();t.exports=n(function(e,t,n){return e+(n?` `:``)+r(t)})}));e(Qn(),1),e(W(),1);function $n(e){let t=(0,J.c)(4),{hostId:n}=e,{data:r}=S(z,n),i;t[0]===r?i=t[1]:(i=r===void 0?[]:r,t[0]=r,t[1]=i);let a=i,o;return t[2]===a?o=t[3]:(o=a.some(er),t[2]=a,t[3]=o),o}function er(e){return e.name===`apps`&&e.enabled}var tr=1e3,nr=8,rr=[`connector-logo-src`],ir=[`connector-logos`],ar=[`apps`,`list`],or=k(E,e=>sr(e));function sr(e){return re({queryKey:vr(e),queryFn:async()=>_r({forceRefetch:!1,hostId:e}),notifyOnChangeProps:[`data`,`dataUpdatedAt`,`error`,`fetchStatus`,`status`],retry:!1,staleTime:D.FIVE_MINUTES})}async function cr({hostId:e,queryClient:t}){let n=await _r({forceRefetch:!0,hostId:e});return t.setQueryData(vr(e),n),n}function lr(e){let t=(0,J.c)(23),n;t[0]===e?n=t[1]:(n=e===void 0?{}:e,t[0]=e,t[1]=n);let{enabled:r,hostId:i}=n,a=r===void 0?!0:r,o=i??`local`,s=H(),c;t[2]===o?c=t[3]:(c={hostId:o},t[2]=o,t[3]=c);let l=$n(c),u=A(),d=!s.isLoading&&s.userId!=null,f;t[4]===o?f=t[5]:(f=sr(o),t[4]=o,t[5]=f);let p=a&&l&&d,m;t[6]!==f||t[7]!==p?(m={...f,enabled:p,staleTime:D.FIVE_MINUTES},t[6]=f,t[7]=p,t[8]=m):m=t[8];let h=j(m),g;t[9]!==o||t[10]!==u?(g={retry:!1,onMutate:async()=>{await u.cancelQueries({queryKey:vr(o)})},mutationFn:async()=>cr({hostId:o,queryClient:u})},t[9]=o,t[10]=u,t[11]=g):g=t[11];let _=N(g),v=_.error!=null&&_.submittedAt>h.dataUpdatedAt?_.error:null,y;t[12]!==l||t[13]!==h?(y=l?h.data:[],t[12]=l,t[13]=h,t[14]=y):y=t[14];let b;t[15]===_?b=t[16]:(b=async()=>_.mutateAsync(),t[15]=_,t[16]=b);let x=v??h.error??null,S;return t[17]!==_.isPending||t[18]!==h||t[19]!==x||t[20]!==y||t[21]!==b?(S={...h,data:y,hardRefetchAppsList:b,isHardRefetchingAppsList:_.isPending,loadError:x},t[17]=_.isPending,t[18]=h,t[19]=x,t[20]=y,t[21]=b,t[22]=S):S=t[22],S}function ur(e){let t=(0,J.c)(14),{apps:n,enabled:r}=e,i=r===void 0?!0:r,a=A(),o=i?n:void 0,s;t[0]===o?s=t[1]:(s=o==null?void 0:br(o),t[0]=o,t[1]=s);let c=s,l;t[2]===c?l=t[3]:(l=[...ir,...(c??[]).map(dr)],t[2]=c,t[3]=l);let u=l,d;t[4]!==c||t[5]!==a?(d=async()=>{if(c==null)throw Error(`connector logo requests are required`);return xr({queryClient:a,requests:c})},t[4]=c,t[5]=a,t[6]=d):d=t[6];let f=c!=null&&c.length>0,p;t[7]!==u||t[8]!==d||t[9]!==f?(p={queryKey:u,queryFn:d,enabled:f,staleTime:D.INFINITE},t[7]=u,t[8]=d,t[9]=f,t[10]=p):p=t[10];let m=j(p),h;bb0:{if(o==null){h=void 0;break bb0}let e;t[11]!==o||t[12]!==m.data?(e=Sr({apps:o,connectorLogoSrcByCacheKey:m.data}),t[11]=o,t[12]=m.data,t[13]=e):e=t[13],h=e}return h}function dr(e){return fn(e)}function fr(e){let t=(0,J.c)(2),n;t[0]===e?n=t[1]:(n=yr(e),t[0]=e,t[1]=n);let{data:r}=j(n);return r??void 0}function pr(e){let t=(0,J.c)(8),n;t[0]===e?n=t[1]:(n=e===void 0?{}:e,t[0]=e,t[1]=n);let{hostId:r}=n,i;t[2]===r?i=t[3]:(i={hostId:r},t[2]=r,t[3]=i);let{data:a}=lr(i),o;t[4]===a?o=t[5]:(o=a===void 0?[]:a,t[4]=a,t[5]=o);let s=o,c;return t[6]===s?c=t[7]:(c=hr(s),t[6]=s,t[7]=c),c}function mr(e){let t=(0,J.c)(4),n;t[0]===e?n=t[1]:(n=e===void 0?{}:e,t[0]=e,t[1]=n);let{hostId:r}=n,i;t[2]===r?i=t[3]:(i={hostId:r},t[2]=r,t[3]=i);let a=pr(i);return ur({apps:a})??a}function hr(e){return e.filter(e=>e.isAccessible&&e.isEnabled)}async function gr({forceRefetch:e,hostId:t}){try{let n=async r=>{let i=await L(`list-apps`,{hostId:t,cursor:r,limit:tr,forceRefetch:e});return i.nextCursor==null?i.data:[...i.data,...await n(i.nextCursor)]};return Sr({apps:await n(null)})}catch(e){throw P.error(`Failed to load apps list`,{safe:{error:String(e)},sensitive:{}}),e instanceof Error?e:Error(String(e))}}async function _r({forceRefetch:e,hostId:t}){try{return await gr({forceRefetch:e,hostId:t})}catch{return gr({forceRefetch:e,hostId:t})}}function vr(e){return[...ar,e]}function yr(e){return{queryKey:[...rr,fn(e)],queryFn:async()=>{try{return await pn(e)}catch{return null}},retry:!1,staleTime:D.INFINITE}}function br(e){let t=new Map;return e.forEach(e=>{let{logoUrl:n,logoUrlDark:r}=Cr(e),i=dn(n);i!=null&&t.set(fn(i),i);let a=dn(r);a!=null&&t.set(fn(a),a)}),Array.from(t.values())}async function xr({queryClient:e,requests:t}){let n=new Map,r=0;return await Promise.all(Array.from({length:Math.min(t.length,nr)},async()=>{for(;;){let i=t[r];if(r+=1,i==null)return;let a=await e.fetchQuery(yr(i));a!=null&&n.set(fn(i),a)}})),n}function Sr({apps:e,connectorLogoSrcByCacheKey:t}){let n=!1,r=e.map(e=>{let r=Cr(e),i=wr({logoUrl:r.logoUrl,installUrl:e.installUrl,connectorLogoSrcByCacheKey:t}),a=wr({logoUrl:r.logoUrlDark,installUrl:e.installUrl,connectorLogoSrcByCacheKey:t});return i===e.logoUrl&&a===e.logoUrlDark?e:(n=!0,{...e,logoUrl:i,logoUrlDark:a})});return n?r:e}function Cr(e){let t=e.iconAssets?.[`256_square`],n=e.iconDarkAssets?.[`256_square`];if(t==null&&n==null)return{logoUrl:e.logoUrl,logoUrlDark:e.logoUrlDark};let r=t??e.logoUrl??n??e.logoUrlDark;return{logoUrl:r,logoUrlDark:n??e.logoUrlDark??t??r}}function wr({logoUrl:e,installUrl:t,connectorLogoSrcByCacheKey:n}){let r=e?.trim();if(r==null||r.length===0)return null;let i=Tr({logoUrl:r,installUrl:t});if(n==null)return i;let a=dn(i);return a==null?i:n.get(fn(a))??i}function Tr({logoUrl:e,installUrl:t}){if(!e.startsWith(`/`))return e;let n=t?.trim();if(n==null||n.length===0)return e;try{return new URL(e,n).toString()}catch{return e}}function Er(e){let t=(0,J.c)(29),{alt:n,appInfo:r,className:i,knownAppId:a,loadRemote:o,logoUrl:s,logoDarkUrl:c,fallback:l}=e,u=o===void 0?!0:o,d=pe()===!0,f,p,m,h,_;if(t[0]!==r||t[1]!==i||t[2]!==l||t[3]!==d||t[4]!==a||t[5]!==u||t[6]!==c||t[7]!==s){_=Symbol.for(`react.early_return_sentinel`);bb0:{p=g(`rounded-2xs`,i);let e;t[13]!==r||t[14]!==a?(e=Dr({appInfo:r,knownAppId:a}),t[13]=r,t[14]=a,t[15]=e):e=t[15];let n=e;if(n!=null){let e;t[16]!==n||t[17]!==i?(e=(0,$.jsx)(n,{"aria-hidden":!0,className:i}),t[16]=n,t[17]=i,t[18]=e):e=t[18],_=e;break bb0}if(m=u?Or({logoUrl:s??r?.logoUrl,logoDarkUrl:c??r?.logoUrlDark,isDarkTheme:d}):null,f=(0,me.cloneElement)(l,{className:g(p,l.props.className)}),m==null){_=f;break bb0}h=dn(m)}t[0]=r,t[1]=i,t[2]=l,t[3]=d,t[4]=a,t[5]=u,t[6]=c,t[7]=s,t[8]=f,t[9]=p,t[10]=m,t[11]=h,t[12]=_}else f=t[8],p=t[9],m=t[10],h=t[11],_=t[12];if(_!==Symbol.for(`react.early_return_sentinel`))return _;let v=h;if(v!=null){let e;return t[19]!==n||t[20]!==v||t[21]!==f||t[22]!==p?(e=(0,$.jsx)(Ar,{className:p,alt:n,connectorLogoRequest:v,fallback:f}),t[19]=n,t[20]=v,t[21]=f,t[22]=p,t[23]=e):e=t[23],e}let y;return t[24]!==n||t[25]!==f||t[26]!==p||t[27]!==m?(y=(0,$.jsx)(jr,{alt:n,className:p,src:m,fallback:f},m),t[24]=n,t[25]=f,t[26]=p,t[27]=m,t[28]=y):y=t[28],y}function Dr({appInfo:e,knownAppId:t}){if(e!=null){let t=an(e);if(t!=null)return t}return t==null?null:rn(t)}function Or({logoUrl:e,logoDarkUrl:t,isDarkTheme:n}){let r=kr(e),i=kr(t);return n?i??r:r??i}function kr(e){let t=e?.trim();return t==null||t.length===0?null:t}function Ar(e){let t=(0,J.c)(5),{className:n,alt:r,connectorLogoRequest:i,fallback:a}=e,o=fr(i);if(o==null)return a;let s;return t[0]!==r||t[1]!==n||t[2]!==a||t[3]!==o?(s=(0,$.jsx)(jr,{className:n,alt:r,fallback:a,src:o},o),t[0]=r,t[1]=n,t[2]=a,t[3]=o,t[4]=s):s=t[4],s}function jr(e){let t=(0,J.c)(7),{alt:n,className:r,src:i,fallback:a}=e,[o,s]=(0,me.useState)(!1);if(o)return a;let c;t[0]===r?c=t[1]:(c=g(`object-contain`,r),t[0]=r,t[1]=c);let l;t[2]===Symbol.for(`react.memo_cache_sentinel`)?(l=()=>{s(!0)},t[2]=l):l=t[2];let u;return t[3]!==n||t[4]!==i||t[5]!==c?(u=(0,$.jsx)(`img`,{alt:n,className:c,src:i,onError:l}),t[3]=n,t[4]=i,t[5]=c,t[6]=u):u=t[6],u}var Mr=T({kind:`closed`}),Nr=T(null,(e,t,{hostId:n,pluginId:r})=>{let i=e(Mr);return i.kind!==`installing`||i.hostId!==n||i.plugin.plugin.id!==r||i.installStarted?!1:(t(Mr,{...i,installStarted:!0}),!0)});function Pr(){let e=(0,J.c)(45),[t,n]=ne(Mr),r=te(Nr),i;e[0]===n?i=e[1]:(i=(e,t,r)=>{let i=r===void 0?{}:r;n(n=>n.kind===`installing`||n.kind===`connectAppBeforeInstall`?n:Rr(e,{...i,plugin:t}))},e[0]=n,e[1]=i);let a=i,o;e[2]===n?o=e[3]:(o=(e,t,r)=>{let i=r===void 0?{}:r;n(n=>n.kind===`installing`||n.kind===`preparingApp`||n.kind===`connectAppBeforeInstall`?n:{...i,kind:`preparingApp`,hostId:e,plugin:t})},e[2]=n,e[3]=o);let s=o,c;e[4]===n?c=e[5]:(c=e=>{let{app:t,hostId:r,options:i,plugin:a}=e;n(e=>e.kind===`preparingApp`&&e.hostId===r&&e.plugin.plugin.id===a.plugin.id?{...i,kind:`connectAppBeforeInstall`,app:t,hostId:r,plugin:a,status:`pending`}:e)},e[4]=n,e[5]=c);let l=c,u;e[6]===n?u=e[7]:(u=e=>{let{hostId:t,pluginId:r}=e;n(e=>e.kind===`preparingApp`&&e.hostId===t&&e.plugin.plugin.id===r?{kind:`closed`}:e)},e[6]=n,e[7]=u);let d=u,f;e[8]===n?f=e[9]:(f=e=>{let{hostId:t,pluginId:r}=e;n(e=>e.kind===`preparingApp`&&e.hostId===t&&e.plugin.plugin.id===r?Rr(t,e):e)},e[8]=n,e[9]=f);let p=f,m;e[10]===n?m=e[11]:(m=e=>{let{appId:t,hostId:r,pluginId:i}=e;n(e=>e.kind===`connectAppBeforeInstall`&&e.app.id===t&&e.hostId===r&&e.plugin.plugin.id===i?Rr(r,e):e)},e[10]=n,e[11]=m);let h=m,g;e[12]===n?g=e[13]:(g=(e,t)=>{n({...t===void 0?{}:t,kind:`details`,plugin:e})},e[12]=n,e[13]=g);let _=g,v;e[14]===n?v=e[15]:(v=()=>{n({kind:`closed`})},e[14]=n,e[15]=v);let y=v,b;e[16]===n?b=e[17]:(b=e=>{n(t=>t.kind===`installing`?{...t,progressPercent:e}:t)},e[16]=n,e[17]=b);let x=b,S;e[18]===n?S=e[19]:(S=e=>{let{apps:t,browserExtensions:r,connectingAppId:i,options:a,plugin:o}=e,s=t.map(Lr),c=s.find(e=>e.app.id===i);if(c==null){n({...a,kind:`needsApps`,plugin:o,requiredApps:s,requiredBrowserExtensions:r});return}n({...a,kind:`connectApp`,plugin:o,app:c.app,requiredApps:s,requiredBrowserExtensions:r})},e[18]=n,e[19]=S);let C=S,w;e[20]===n?w=e[21]:(w=e=>{n(t=>{if(t.kind!==`needsApps`)return t;let n=t.requiredApps.find(t=>t.app.id===e);return n==null?t:{...t,kind:`connectApp`,app:n.app}})},e[20]=n,e[21]=w);let T=w,E;e[22]===n?E=e[23]:(E=()=>{n(Ir)},e[22]=n,e[23]=E);let D=E,O;e[24]===n?O=e[25]:(O=()=>{n(Fr)},e[24]=n,e[25]=O);let k=O,A;e[26]===n?A=e[27]:(A=e=>{let{appId:t,status:r}=e;n(e=>e.kind===`connectAppBeforeInstall`?e.app.id===t?{...e,status:r}:e:e.kind!==`needsApps`&&e.kind!==`connectApp`?e:{...e,requiredApps:e.requiredApps.map(e=>e.app.id===t?{...e,status:r}:e)})},e[26]=n,e[27]=A);let j=A,M;return e[28]!==d||e[29]!==r||e[30]!==y||e[31]!==D||e[32]!==k||e[33]!==j||e[34]!==a||e[35]!==l||e[36]!==_||e[37]!==T||e[38]!==s||e[39]!==t||e[40]!==C||e[41]!==x||e[42]!==h||e[43]!==p?(M={cancelPluginInstallAppPreparation:d,claimPluginInstall:r,closePluginInstallAppConnectBeforeInstall:k,closePluginInstallAppConnect:D,closePluginInstall:y,markRequiredAppStatus:j,openPluginInstallDetails:_,openPluginInstall:a,openPluginInstallAppConnectBeforeInstall:l,openRequiredAppConnect:T,preparePluginInstallAppConnect:s,session:t,setPluginInstallProgress:x,setPluginInstallNeedsApps:C,startPluginInstallAfterAppPreparation:p,startPluginInstallAfterAppConnect:h},e[28]=d,e[29]=r,e[30]=y,e[31]=D,e[32]=k,e[33]=j,e[34]=a,e[35]=l,e[36]=_,e[37]=T,e[38]=s,e[39]=t,e[40]=C,e[41]=x,e[42]=h,e[43]=p,e[44]=M):M=e[44],M}function Fr(e){return e.kind===`connectAppBeforeInstall`?{kind:`closed`}:e}function Ir(e){return e.kind===`connectApp`?e.requiredApps.length===1&&e.requiredBrowserExtensions.length===0&&e.requiredApps[0]?.status!==`connected`&&e.requiredApps[0]?.status!==`waitingForCallback`?{kind:`closed`}:{kind:`needsApps`,marketplaceAnalytics:e.marketplaceAnalytics,origin:e.origin,postInstallComposerPrefill:e.postInstallComposerPrefill,postInstallNewConversation:e.postInstallNewConversation,plugin:e.plugin,requiredApps:e.requiredApps,requiredBrowserExtensions:e.requiredBrowserExtensions,activeProject:e.activeProject,telemetry:e.telemetry}:e}function Lr(e){return{app:e,status:`pending`}}function Rr(e,t){return{...t,kind:`installing`,hostId:e,installStarted:!1,phase:`installing`,progressPercent:0}}export{Y as $,Yt as A,lt as B,xn as C,Qt as D,an as E,Z as F,wt as G,kt as H,rt as I,Be as J,dt as K,Vt as L,Kt as M,Gt as N,Zt as O,Wt as P,ge as Q,at as R,bn as S,rn as T,Ot as U,st as V,Dt as W,Se as X,Te as Y,be as Z,wn as _,hr as a,Tn as b,ur as c,mr as d,$n as f,vn as g,Wn as h,vr as i,Jt as j,Xt as k,fr as l,Zn as m,Er as n,cr as o,Qn as p,Ve as q,or as r,lr as s,Pr as t,pr as u,Sn as v,dn as w,yn as x,Cn as y,it as z};