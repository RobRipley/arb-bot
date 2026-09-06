const fs=require('node:fs'), vm=require('node:vm'), assert=require('node:assert/strict');
const html=fs.readFileSync('src/arb_bot/src/dashboard.html','utf8');
const code=html.slice(html.indexOf('    function routeTradingLabel()'),html.indexOf('    function routeArbitrageHtml()'));
let modal; const calls=[];
const status={compiled_support:true,live_authorized:false,enabled:true,dry_run:true,last_error:[],last_realized_profit:[],last_profit_class:[]};
const ctx=vm.createContext({latestRouteRuntime:null,window:{},isAdmin:true,routeOpt:x=>x?.[0]??null,esc:String,variantKey:x=>Object.keys(x)[0],
  requireAuth:()=>true,openModal:x=>{modal=x},unwrapResult:x=>{if('Err'in x)throw Error(x.Err);return x.Ok},
  authenticatedActor:{get_route_arb_config_v1:async()=>({enabled:false,dry_run:true,max_route_legs:3}),
    set_route_arb_config_v1:async c=>{calls.push(['config',c]);return {Ok:null}},
    set_route_runtime_authorized_v1:async enabled=>{calls.push(['authorize',enabled]);return {Ok:{...status,live_authorized:enabled,dry_run:false}}}},
  loadRouteData:async()=>{},renderAll(){},toast(){}});
vm.runInContext(code,ctx);
(async()=>{
  assert.equal(vm.runInContext('routeTradingLabel()',ctx),'Status unavailable');
  assert(!vm.runInContext('routeRuntimeHtml()',ctx).includes('onclick='));
  ctx.latestRouteRuntime=status;
  assert.equal(vm.runInContext('routeTradingLabel()',ctx),'Trading stopped');
  vm.runInContext('window.setRouteTrading(true)',ctx); assert.equal(calls.length,0);
  await modal.onConfirm();
  assert.equal(calls[0][0],'config');assert.equal(calls[0][1].enabled,true);assert.equal(calls[0][1].dry_run,false);
  assert.equal(calls[0][1].max_route_legs,3);assert.deepEqual(calls[1],['authorize',true]);
  assert.equal(vm.runInContext('routeTradingLabel()',ctx),'Automatic arbitrage on');
  calls.length=0;vm.runInContext('window.setRouteTrading(false)',ctx);await modal.onConfirm();
  assert.deepEqual(calls,[['authorize',false]]);
  calls.length=0;ctx.authenticatedActor.set_route_arb_config_v1=async()=>({Err:'changed policy'});
  vm.runInContext('window.setRouteTrading(true)',ctx);await modal.onConfirm();assert.equal(calls.length,0);
  console.log('PASS: runtime UI distinguishes unknown/off/on; configuration failure cannot authorize; stop disables new trades');
})().catch(e=>{console.error(e);process.exitCode=1});
