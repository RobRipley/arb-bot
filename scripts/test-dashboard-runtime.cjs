const fs=require('node:fs'), vm=require('node:vm'), assert=require('node:assert/strict');
const html=fs.readFileSync('src/arb_bot/src/dashboard.html','utf8');
const code=html.slice(html.indexOf('    function routeTradingLabel()'),html.indexOf('    function routeArbitrageHtml()'));
let modal; const calls=[]; const toasts=[]; let resolveConfig;
const status={compiled_support:true,live_authorized:false,enabled:true,dry_run:true,last_error:[],last_realized_profit:[],last_profit_class:[]};
const ctx=vm.createContext({latestRouteRuntime:null,routeSources:{runtime:{state:'loading'}},ROUTE_RUNTIME_STALE_AFTER_MS:30000,state:{levers:{}},window:{},isAdmin:true,routeOpt:x=>x?.[0]??null,esc:String,variantKey:x=>Object.keys(x)[0],
  routeSourceState:()=>ctx.routeSources.runtime.state || (ctx.latestRouteRuntime?'fresh':'loading'),sourceLastSuccessLabel:()=> 'just now',
  requireAuth:()=>true,openModal:x=>{modal=x},unwrapResult:x=>{if('Err'in x)throw Error(x.Err);return x.Ok},
  authenticatedActor:{get_route_arb_config_v1:async()=>({enabled:false,dry_run:true,max_route_legs:3}),
    set_route_arb_config_v1:async c=>{calls.push(['config',c]);return new Promise(resolve=>{resolveConfig=()=>resolve({Ok:null})})},
    set_route_runtime_authorized_v1:async enabled=>{calls.push(['authorize',enabled]);return {Ok:{...status,live_authorized:enabled,dry_run:false}}}},
  loadRouteData:async()=>{ctx.latestRouteRuntime={...status,live_authorized:calls.some(c=>c[0]==='authorize'&&c[1]),dry_run:false};ctx.routeSources.runtime.state='fresh'},renderAll(){},toast:(message,kind)=>toasts.push([message,kind])});
vm.runInContext(code,ctx);
(async()=>{
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Unknown');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'Unknown');
  assert(!vm.runInContext('routeRuntimeHtml()',ctx).includes('onclick='));
  ctx.latestRouteRuntime=status;
  ctx.routeSources.runtime.state='fresh';
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Off');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'Stopped');
  assert(vm.runInContext('routeRuntimeHtml()',ctx).includes('aria-pressed="false"'));
  vm.runInContext('window.setRouteTrading(true)',ctx); assert.equal(calls.length,0);
  const pending=modal.onConfirm();
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Applying');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'Starting');
  await new Promise(resolve => setImmediate(resolve));
  resolveConfig();
  await pending;
  assert.equal(calls[0][0],'config');assert.equal(calls[0][1].enabled,true);assert.equal(calls[0][1].dry_run,false);
  assert.equal(calls[0][1].max_route_legs,3);assert.deepEqual(calls[1],['authorize',true]);
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'On');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'On');
  assert(toasts.some(([message,kind])=>kind==='success' && message.includes('confirmed by runtime refresh')));
  calls.length=0;vm.runInContext('window.setRouteTrading(false)',ctx);await modal.onConfirm();
  assert.deepEqual(calls,[['authorize',false]]);
  calls.length=0;ctx.authenticatedActor.set_route_arb_config_v1=async()=>({Err:'changed policy'});
  vm.runInContext('window.setRouteTrading(true)',ctx);await modal.onConfirm();assert.equal(calls.length,0);
  ctx.latestRouteRuntime={...status,live_authorized:true,dry_run:false};ctx.routeSources.runtime.state='stale';
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Blocked');
  console.log('PASS: runtime UI distinguishes unknown/off/applying/on/stopping/blocked; configuration failure cannot authorize; stop disables new trades');
})().catch(e=>{console.error(e);process.exitCode=1});
