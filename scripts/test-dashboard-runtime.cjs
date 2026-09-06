const fs=require('node:fs'), vm=require('node:vm'), assert=require('node:assert/strict');
const html=fs.readFileSync('src/arb_bot/src/dashboard.html','utf8');
const helperCode=html.slice(html.indexOf('    function routeRuntimeInFlightState('),html.indexOf('    function routeCandidateQuoteState('));
const code=helperCode+html.slice(html.indexOf('    function routeTradingLabel()'),html.indexOf('    function routeArbitrageHtml()'));
const freshnessCode=html.slice(html.indexOf('    function routeTimestampMs('),html.indexOf('    function routeQuoteStaleAfterMs('));
let modal; const calls=[]; const events=[]; const toasts=[]; let resolveConfig; let drainResolve;
const status={compiled_support:true,live_authorized:false,enabled:true,dry_run:true,last_error:[],last_realized_profit:[],last_profit_class:[],scheduler_in_flight_since_ns:[]};
const ctx=vm.createContext({latestRouteRuntime:null,routeSources:{runtime:{state:'loading'}},ROUTE_RUNTIME_STALE_AFTER_MS:30000,SCHEDULER_NO_PROGRESS_CEILING_MS:600000,state:{levers:{}},routeDataRequestPromise:null,routeRuntimeQueryGeneration:0,routeRuntimeAuthoritativeRefreshPromise:null,window:{},isAdmin:true,routeOpt:x=>x?.[0]??null,routeTimestampMs:x=>Number(x)/1e6,routeAge:()=> '2.0m',esc:String,variantKey:x=>Object.keys(x)[0],
  routeSourceState:()=>ctx.routeSources.runtime.state || (ctx.latestRouteRuntime?'fresh':'loading'),sourceLastSuccessLabel:()=> 'just now',
  routeRuntimePayloadTimestampMs:()=>Date.now(),
  markSourceFresh:(source,value)=>{source.state='fresh';source.value=value},markSourceFailed:(source,error)=>{source.state='failed';source.error=String(error)},markSourceUnavailable:(source,error)=>{source.state='unavailable';source.error=String(error)},
  requireAuth:()=>true,openModal:x=>{modal=x},unwrapResult:x=>{if('Err'in x)throw Error(x.Err);return x.Ok},
  authenticatedActor:{get_route_arb_config_v1:async()=>({enabled:false,dry_run:true,max_route_legs:3}),
    set_route_arb_config_v1:async c=>{calls.push(['config',c]);events.push('config');return new Promise(resolve=>{resolveConfig=()=>resolve({Ok:null})})},
    set_route_runtime_authorized_v1:async enabled=>{calls.push(['authorize',enabled]);events.push('authorize');return {Ok:{...status,live_authorized:enabled,dry_run:false}}}},
  anonymousActor:{get_route_runtime_status_v1:async()=>{events.push('runtime-query');return {Ok:{...status,live_authorized:events.includes('authorize'),dry_run:false}}}},
  loadRouteData:async()=>{ctx.latestRouteRuntime={...status,live_authorized:calls.some(c=>c[0]==='authorize'&&c[1]),dry_run:false};ctx.routeSources.runtime.state='fresh'},renderAll(){events.push(`renderAll:${ctx.state.levers.routeAutomation||'clear'}`)},toast:(message,kind)=>toasts.push([message,kind])});
vm.runInContext(code,ctx);
(async()=>{
  const nowMs=1_000_000;
  const freshCtx=vm.createContext({Date:{now:()=>nowMs},routeOpt:x=>x?.[0]??null,SCHEDULER_NO_PROGRESS_CEILING_MS:600000});
  vm.runInContext(freshnessCode,freshCtx);
  assert.equal(vm.runInContext('routeRuntimePayloadTimestampMs({last_tick_ns:1n,scheduler_in_flight_since_ns:[900000000000n],live_authorized:true,enabled:true,dry_run:false},1000000)',freshCtx),1000000);
  assert.equal(vm.runInContext('routeRuntimePayloadTimestampMs({last_tick_ns:500000000000n,scheduler_in_flight_since_ns:[100000000000n],live_authorized:true,enabled:true,dry_run:false},1000000)',freshCtx),500000);
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Unknown');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'Unknown');
  assert(!vm.runInContext('routeRuntimeHtml()',ctx).includes('onclick='));
  ctx.latestRouteRuntime=status;
  ctx.routeSources.runtime.state='fresh';
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Off');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'Off');
  assert(vm.runInContext('routeRuntimeHtml()',ctx).includes('aria-pressed="false"'));
  assert(vm.runInContext('routeRuntimeHtml()',ctx).includes('>Off<'));
  vm.runInContext('window.setRouteTrading(true)',ctx); assert.equal(calls.length,0);
  const pending=modal.onConfirm();
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Applying');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'Applying');
  ctx.routeDataRequestPromise = new Promise(resolve => { drainResolve = resolve; });
  await new Promise(resolve => setImmediate(resolve));
  resolveConfig();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(events.slice(0,2),['config','authorize']);
  assert(!events.includes('runtime-query'), 'authoritative query must wait for pre-mutation load to drain');
  drainResolve();
  await pending;
  assert.equal(calls[0][0],'config');assert.equal(calls[0][1].enabled,true);assert.equal(calls[0][1].dry_run,false);
  assert.equal(calls[0][1].max_route_legs,3);assert.deepEqual(calls[1],['authorize',true]);
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'On');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'On');
  ctx.latestRouteRuntime={...ctx.latestRouteRuntime,scheduler_in_flight_since_ns:[BigInt(Date.now())*1000000n]};
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'On');
  assert.equal(vm.runInContext('routeAutomationState().label',ctx),'Scanning');
  assert(events.indexOf('runtime-query') > events.indexOf('authorize'), 'runtime query must start after authorization mutation');
  assert(events.some(event => event.startsWith('renderAll:')), 'clear Applying must repaint the mounted views');
  assert(toasts.some(([message,kind])=>kind==='success' && message.includes('confirmed by runtime refresh')));
  calls.length=0;vm.runInContext('window.setRouteTrading(false)',ctx);await modal.onConfirm();
  assert.deepEqual(calls,[['authorize',false]]);
  calls.length=0;ctx.authenticatedActor.set_route_arb_config_v1=async()=>({Err:'changed policy'});
  vm.runInContext('window.setRouteTrading(true)',ctx);await modal.onConfirm();assert.equal(calls.length,0);
  assert.equal(events.filter(event => event.startsWith('renderAll:')).at(-1), 'renderAll:clear', 'failure path must repaint after clearing Applying');
  ctx.latestRouteRuntime={...status,live_authorized:true,dry_run:false};ctx.routeSources.runtime.state='stale';
  assert.equal(vm.runInContext('routeAutomationState().state',ctx),'Blocked');
  console.log('PASS: runtime UI distinguishes unknown/off/applying/on/stopping/blocked; configuration failure cannot authorize; stop disables new trades');
})().catch(e=>{console.error(e);process.exitCode=1});
