// Simple SPA, vanilla JS
const API = {
  async call(path, opts={}){
    opts.credentials = 'include'; // needed for cookies
    try{
      let r = await fetch(path, opts);
      if(r.status===401||r.status===403){
        // try to read any message body and include it so frontend can show a popup
        let txt = await r.text();
        let parsed;
        try{ parsed = JSON.parse(txt); }catch(e){ parsed = txt; }
        throw {unauth:true, message: parsed};
      }
      let txt = await r.text();
      try{ return JSON.parse(txt); }catch(e){ return txt; }
    }catch(e){
      if(e.unauth) throw e;
      throw e;
    }
  }
}

// timezone helper: get checkpoint date (UTC+8, 4:00 cutoff)
function getCheckpointDateFor(ts){
  // ts is Date
  // convert to UTC+8
  const utc = ts.getTime() + ts.getTimezoneOffset()*60000;
  const tz8 = new Date(utc + 8*3600*1000);
  // if time >= 04:00, use its date; otherwise use previous date
  if(tz8.getHours() >= 4){
    return new Date(tz8.getFullYear(), tz8.getMonth(), tz8.getDate());
  }else{
    const d = new Date(tz8.getFullYear(), tz8.getMonth(), tz8.getDate()-1);
    return d;
  }
}

function formatDate(d){
  return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`;
}

let state = { date: getCheckpointDateFor(new Date()) };

function showAuth(){ document.getElementById('auth').classList.remove('hidden'); }
function hideAuth(){ document.getElementById('auth').classList.add('hidden'); }
function showReset(){ document.getElementById('reset').classList.remove('hidden'); }
function hideReset(){ document.getElementById('reset').classList.add('hidden'); }

async function loadRecords(){
  document.getElementById('date').textContent = formatDate(state.date);
  try{
    const dateStr = formatDate(state.date);
    const res = await API.call(`/daka/records?date=${encodeURIComponent(dateStr)}`);
    // assume res.records is a string or array
    const list = document.getElementById('list');
    list.innerHTML = '';
    if(typeof res.records === 'string'){
      const li = document.createElement('li'); li.textContent = res.records; list.appendChild(li);
    }else if(Array.isArray(res.records)){
      res.records.forEach(r=>{
        const li = document.createElement('li');
        // r is { name, time }
        if(typeof r === 'object' && r !== null){
          const timeTxt = r.time ? r.time : '❌';
          li.textContent = `${r.name} — ${timeTxt}`;
        }else{
          li.textContent = String(r);
        }
        list.appendChild(li);
      });
    }else{
      const li = document.createElement('li'); li.textContent = JSON.stringify(res); list.appendChild(li);
    }

  // popup any backend message (e.g., res.message or res.error)
    if(res && res.message){ alert(res.message); }
    if(res && res.error){ alert(res.error); }

    // disable next button if viewing today
    const today = getCheckpointDateFor(new Date());
    const nextBtn = document.getElementById('next');
    if(formatDate(today) === formatDate(state.date)){
      nextBtn.setAttribute('disabled','disabled');
      nextBtn.classList.add('today');
    }else{
      nextBtn.removeAttribute('disabled');
      nextBtn.classList.remove('today');
    }

  // show/hide action buttons depending on whether viewing today
  const isToday = (formatDate(today) === formatDate(state.date));
  const dakaBtn = document.getElementById('daka');
  const undoBtn = document.getElementById('undo');
  if(isToday){ dakaBtn.style.display = ''; undoBtn.style.display = ''; }
  else { dakaBtn.style.display = 'none'; undoBtn.style.display = 'none'; }
  // hide entire footer when not today
  const footerEl = document.querySelector('footer');
  if(isToday){
    document.body.classList.remove('footer-hidden');
    if(footerEl) footerEl.style.display = '';
  } else {
    document.body.classList.add('footer-hidden');
    if(footerEl) footerEl.style.display = 'none';
  }
  // check gu button state after loading records
  checkAndShowGuButton();
  }catch(e){
    if(e.unauth){ showAuth(); return; }
    console.error(e); alert('load failed');
  }
}

// 咕 button logic: visible only on today and when either array is non-empty
async function checkAndShowGuButton(){
  const guBtn = document.getElementById('gu');
  const today = getCheckpointDateFor(new Date());
  const isToday = (formatDate(today) === formatDate(state.date));
  if(!isToday){ guBtn.style.display = 'none'; return; }
  try{
    const res = await API.call('/daka/gu');
    const hasAny = (Array.isArray(res.missed_10) && res.missed_10.length>0) || (Array.isArray(res.warning_7) && res.warning_7.length>0);
    if(hasAny){ guBtn.style.display = ''; } else { guBtn.style.display = 'none'; }
    // store last fetched results for modal
    guBtn._last = res;
  }catch(e){ if(e.unauth){ guBtn.classList.add('hidden'); } else { console.error('gu fetch failed', e); guBtn.classList.add('hidden'); } }
}

function showGuModal(){
  const guBtn = document.getElementById('gu');
  const res = guBtn._last || { missed_10: [], warning_7: [] };
  const missList = document.getElementById('gu-missed-list');
  const warnList = document.getElementById('gu-warning-list');
  missList.innerHTML = '';
  warnList.innerHTML = '';
  const missed = (res.missed_10 || []);
  const warning = (res.warning_7 || []);
  if(missed.length>0){
    document.getElementById('gu-missed').style.display = '';
    missed.forEach(n => { const li = document.createElement('li'); li.textContent = n; missList.appendChild(li); });
  } else { document.getElementById('gu-missed').style.display = 'none'; }
  if(warning.length>0){
    document.getElementById('gu-warning').style.display = '';
    warning.forEach(n => { const li = document.createElement('li'); li.textContent = n; warnList.appendChild(li); });
  } else { document.getElementById('gu-warning').style.display = 'none'; }
  document.getElementById('gu-modal').classList.remove('hidden');
}

function hideGuModal(){ document.getElementById('gu-modal').classList.add('hidden'); }

async function doAction(type){
  // type: 'daka' or 'undo'
  try{
    const path = type==='daka'?'/daka/daka':'/daka/daka';
    const method = type==='daka'?'POST':'DELETE';
    const res = await API.call(path, { method, headers: {'Content-Type':'application/json'}, body: JSON.stringify({ }) });
    if(res && res.ok===false && res.need_reset){
      // prompt reset flow
      showReset();
      return;
    }
    // show any message returned by backend even on success
    if(res && res.message){ alert(res.message); }
    if(res && res.error){ alert(res.error); }
    await loadRecords();
  }catch(e){ if(e.unauth){ showAuth(); } else { console.error(e); alert('action failed'); } }
}

async function doLogin(){
  const uin = Number(document.getElementById('uin').value);
  const password = document.getElementById('password').value;
  try{
    const res = await API.call('/login', { method:'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify({ uin, password }) });
    if(res && res.need_reset){ showReset(); hideAuth(); return; }
    hideAuth();
    await loadRecords();
  }catch(e){
    if(e.unauth){
      // if the thrown object includes a message from server, show it
      if(e.message){
        try{ const m = (typeof e.message === 'string') ? e.message : JSON.stringify(e.message); alert(m); }catch(_){ alert('unauthorized'); }
      }
      showAuth();
    } else { alert('login failed'); }
  }
}




async function doSetPassword(){
  const newpass = document.getElementById('newpass').value;
  const uin = Number(document.getElementById('uin').value);
  try{
    const res = await API.call('/reset_password', { method:'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify({ qq_uin: uin, new_password: newpass }) });
    hideReset();
    await loadRecords();
  }catch(e){ if(e.unauth){ showAuth(); } else { alert('set password failed'); } }
}

// UI wiring
window.addEventListener('load', ()=>{
  document.getElementById('prev').addEventListener('click', ()=>{ state.date.setDate(state.date.getDate()-1); loadRecords(); });
  document.getElementById('next').addEventListener('click', ()=>{ state.date.setDate(state.date.getDate()+1); loadRecords(); });
  document.getElementById('gu').addEventListener('click', showGuModal);
  document.getElementById('daka').addEventListener('click', ()=>doAction('daka'));
  document.getElementById('undo').addEventListener('click', ()=>doAction('undo'));
  document.getElementById('login').addEventListener('click', doLogin);
  document.getElementById('setpass').addEventListener('click', doSetPassword);
  document.getElementById('gu-close').addEventListener('click', hideGuModal);
  loadRecords();
  checkAndShowGuButton();
});
