#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod store;
mod jobs;
mod suite_updates;
mod provider_state;

use jobs::{JobManager, SelectedInput, TranslationRequest};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}}};
use tauri::{Emitter, Manager, State};
use versora_core::{Term, glossary::glossary_to_prompt_block};
use versora_engine::providers::{self, CancelToken, ProviderConfig, ProviderStatus};

const VERSION:&str=env!("CARGO_PKG_VERSION");
struct DesktopState {
    store:Arc<store::Store>,jobs:Arc<JobManager>,updates:Arc<suite_updates::UpdateService>,
    selected:Mutex<Vec<SelectedInput>>,output_dirs:Mutex<Vec<PathBuf>>,last_job:Mutex<Option<String>>,
    statuses:Mutex<HashMap<String,ProviderStatus>>,proofs:Mutex<HashMap<String,std::time::Instant>>,pending:AtomicUsize,unsaved:AtomicBool,closing:AtomicBool,ui_ready:AtomicBool,
}
struct Persistence<'a>(&'a AtomicUsize);
impl<'a> Persistence<'a>{fn new(counter:&'a AtomicUsize)->Self{counter.fetch_add(1,Ordering::SeqCst);Self(counter)}}
impl Drop for Persistence<'_>{fn drop(&mut self){self.0.fetch_sub(1,Ordering::SeqCst);}}
fn demo_enabled()->bool{["SFTS_DEMO","VERSORA_DEMO"].iter().any(|v|std::env::var(v).as_deref()==Ok("1"))}

#[tauri::command] fn get_state(state:State<DesktopState>)->Result<Value,String>{
    let prefs=state.store.preferences();let mut rows=state.store.provider_states();let statuses=state.statuses.lock().unwrap();let proofs=state.proofs.lock().unwrap();
    for row in &mut rows{if let Some(status)=row["id"].as_str().and_then(|id|statuses.get(id)){
        provider_state::apply_probe(row,status,proofs.get(&status.id).copied());
    }}
    let last=state.last_job.lock().unwrap().clone();let job=last.as_deref().and_then(|id|state.jobs.snapshot(id).ok());
    Ok(json!({"settings":prefs,"providers":rows,"purposes":state.store.purposes(),"projects":state.store.projects()?,
        "glossary":state.store.glossary(prefs["project"].as_str().unwrap_or("default"))?,"version":VERSION,
        "dataDir":state.store.data_dir().to_string_lossy(),"testMode":demo_enabled(),"selected":state.selected.lock().unwrap().clone(),"job":job}))
}
fn register_paths(paths:Vec<PathBuf>,kind:&str)->Result<Vec<SelectedInput>,String>{
    let mut out=Vec::new();if paths.len()>400{return Err("Choose at most 400 inputs.".into());}
    for path in paths {let meta=std::fs::symlink_metadata(&path).map_err(|e|e.to_string())?;if jobs::is_reparse(&meta){return Err("Symbolic-link or junction inputs are not followed.".into());}
        let path=path.canonicalize().map_err(|e|e.to_string())?;
        if !meta.is_file() && !meta.is_dir(){return Err("Input must be a file or directory.".into());}
        let name=path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        out.push(SelectedInput{path:path.to_string_lossy().into(),name,size:if meta.is_file(){meta.len()}else{0},kind:if meta.is_dir(){"folder".into()}else{kind.into()}});
    }Ok(out)
}
#[tauri::command] fn pick_files(state:State<DesktopState>,kind:String)->Result<Vec<SelectedInput>,String>{
    if state.jobs.active().is_some(){return Err("Wait for the current translation to stop.".into());}
    let paths=match kind.as_str(){"folder"=>rfd::FileDialog::new().pick_folder().into_iter().collect(),"zip"=>rfd::FileDialog::new().add_filter("ZIP",&["zip"]).pick_file().into_iter().collect(),
        "files"=>rfd::FileDialog::new().add_filter("Documents",&["txt","md","markdown","docx","pdf","json","csv","tsv","yaml","yml","po","pot","xliff","xlf","xlsx","html","htm","srt","vtt","lua","js","ts","gd"]).pick_files().unwrap_or_default(),_=>return Err("Unknown input type.".into())};
    let selected=register_paths(paths,&kind)?;*state.selected.lock().unwrap()=selected.clone();*state.last_job.lock().unwrap()=None;Ok(selected)
}
#[tauri::command] fn clear_selection(state:State<DesktopState>)->Result<(),String>{if state.jobs.active().is_some(){return Err("A translation is running.".into());}state.selected.lock().unwrap().clear();*state.last_job.lock().unwrap()=None;Ok(())}
#[tauri::command] fn choose_output_dir(state:State<DesktopState>)->Result<Option<String>,String>{
    if let Some(path)=rfd::FileDialog::new().pick_folder(){let path=path.canonicalize().map_err(|e|e.to_string())?;state.output_dirs.lock().unwrap().push(path.clone());Ok(Some(path.to_string_lossy().into()))}else{Ok(None)}
}
#[tauri::command] fn save_settings(state:State<DesktopState>,app:tauri::AppHandle,settings:Value)->Result<Value,String>{let _pending=Persistence::new(&state.pending);let result=state.store.save_preferences(settings)?;
    if let Some(window)=app.get_webview_window("main"){let _=window.set_theme(Some(if result["theme"]=="dark"{tauri::Theme::Dark}else{tauri::Theme::Light}));}Ok(result)}
#[tauri::command] fn save_provider(state:State<DesktopState>,provider:Value)->Result<Value,String>{let _pending=Persistence::new(&state.pending);let id=provider["id"].as_str().unwrap_or("").to_owned();let result=state.store.save_provider(provider)?;state.statuses.lock().unwrap().remove(&id);state.proofs.lock().unwrap().remove(&id);Ok(result)}
#[tauri::command] fn delete_provider(state:State<DesktopState>,id:String)->Result<Value,String>{let _pending=Persistence::new(&state.pending);let result=state.store.delete_provider(&id)?;state.statuses.lock().unwrap().remove(&id);state.proofs.lock().unwrap().remove(&id);Ok(result)}
fn provider(state:&DesktopState,id:&str)->Result<ProviderConfig,String>{state.store.provider_configs().into_iter().find(|c|c.id==id).ok_or("Unknown translator.".into())}
#[tauri::command] async fn probe_provider(state:State<'_,DesktopState>,id:String)->Result<Value,String>{
    let config=provider(&state,&id)?;let cancel=Arc::new(AtomicBool::new(false));let status=providers::probe(&config,true,&cancel).await;
    state.statuses.lock().unwrap().insert(id,status.clone());serde_json::to_value(status).map_err(|e|e.to_string())
}
#[tauri::command] async fn test_provider(state:State<'_,DesktopState>,id:String)->Result<Value,String>{
    let config=provider(&state,&id)?;state.proofs.lock().unwrap().remove(&id);let cancel=Arc::new(AtomicBool::new(false));
    let text=providers::complete(&config,"Reply with only OK. Do not use any tools.","Connection test: reply OK.",&cancel).await.map_err(|e|e.to_string())?;
    let mut status=providers::probe(&config,true,&cancel).await;status.usable=true;status.detail="The explicit transport Test succeeded; translation quality is not certified.".into();state.statuses.lock().unwrap().insert(id,status);
    state.proofs.lock().unwrap().insert(config.id.clone(),std::time::Instant::now());
    Ok(json!({"ok":true,"reply":text.chars().take(80).collect::<String>()}))
}
#[tauri::command] fn create_project(state:State<DesktopState>,name:String)->Result<String,String>{let _pending=Persistence::new(&state.pending);state.store.create_project(&name)}
#[tauri::command] fn load_glossary(state:State<DesktopState>,project:String)->Result<Vec<Term>,String>{state.store.glossary(&project)}
#[tauri::command] fn save_glossary(state:State<DesktopState>,project:String,entries:Vec<Term>)->Result<Vec<Term>,String>{let _pending=Persistence::new(&state.pending);state.store.save_glossary(&project,entries)}
#[tauri::command] fn save_purpose(state:State<DesktopState>,purpose:Value)->Result<Value,String>{if purpose["id"]!="custom"{return Err("Shipped purpose presets are read-only.".into());}let _pending=Persistence::new(&state.pending);state.store.save_custom(purpose["instructions"].as_str().ok_or("Custom instructions are required.")?)}
#[tauri::command] async fn import_legacy_data(state:State<'_,DesktopState>)->Result<Value,String>{if state.jobs.active().is_some(){return Err("Wait for the current job to stop before importing settings.".into());}
    let _pending=Persistence::new(&state.pending);
    let selected=tokio::task::spawn_blocking(||rfd::FileDialog::new().set_title("選擇舊版 Versora 資料夾").pick_folder()).await.map_err(|e|e.to_string())?;
    let Some(root)=selected else{return Ok(json!({"cancelled":true}));};
    if state.jobs.active().is_some(){return Err("Wait for the current job to stop before importing settings.".into());}
    let store=state.store.clone();tokio::task::spawn_blocking(move||store.import_legacy_data(&root)).await.map_err(|e|e.to_string())?}
fn output_parent(state:&DesktopState,request:&TranslationRequest)->Result<PathBuf,String>{
    let default=state.store.data_dir().join("data/outputs");let Some(raw)=request.output_dir.as_deref().filter(|s|!s.is_empty())else{return Ok(default);};let path=PathBuf::from(raw).canonicalize().map_err(|e|e.to_string())?;
    if path==default.canonicalize().map_err(|e|e.to_string())?||state.output_dirs.lock().unwrap().iter().any(|p|*p==path){Ok(path)}else{Err("Choose the output folder through the native picker.".into())}
}
async fn job_parameters(state:&DesktopState,request:&TranslationRequest)->Result<(Vec<ProviderConfig>,String,usize,usize),String>{
    let prefs=state.store.preferences();let configs=state.store.provider_configs();let mut ordered=Vec::new();
    if request.provider_id=="auto"{
        let chain=prefs["chain"].as_array().cloned().unwrap_or_default();for row in chain{if row["enabled"].as_bool()==Some(false){continue;}if let Some(mut config)=configs.iter().find(|c|row["id"]==c.id).cloned(){
            if let Some(model)=row["model"].as_str().filter(|m|!m.is_empty()){config.model=model.into();}if let Some(effort)=row["effort"].as_str(){config.effort=effort.into();}ordered.push(config);}}
        if demo_enabled()&&!ordered.iter().any(|c|c.id=="demo"){if let Some(demo)=configs.iter().find(|c|c.id=="demo"){ordered.push(demo.clone());}}
    }else{let mut config=configs.into_iter().find(|c|c.id==request.provider_id).ok_or("Unknown translator.")?;
        if let Some(row)=prefs["chain"].as_array().and_then(|rows|rows.iter().find(|row|row["id"]==config.id)){if let Some(model)=row["model"].as_str().filter(|m|!m.is_empty()){config.model=model.into();}if let Some(effort)=row["effort"].as_str(){config.effort=effort.into();}}
        if let Some(model)=request.model.as_deref().filter(|m|!m.is_empty()){if !versora_core::names::valid_model_id(model){return Err("Invalid model identifier.".into());}config.model=model.into();}ordered.push(config);}
    let mut usable=Vec::new();for config in ordered{
        if config.id=="demo"{if demo_enabled(){usable.push(config);}continue;}
        let cancel:CancelToken=Arc::new(AtomicBool::new(false));let status=providers::probe(&config,false,&cancel).await;
        let explicit_proof=state.proofs.lock().unwrap().get(&config.id).is_some_and(|when|when.elapsed()<std::time::Duration::from_secs(300));
        state.statuses.lock().unwrap().insert(config.id.clone(),status.clone());if status.usable||explicit_proof{usable.push(config);}
    }
    let purposes=state.store.purposes();let purpose=purposes.iter().find(|p|p["id"]==request.purpose_id).unwrap_or(&purposes[0]);
    let source=request.source_language.as_deref().filter(|s|!s.is_empty()&&*s!="auto").unwrap_or("the detected source language");
    let glossary=state.store.glossary(request.project.as_deref().unwrap_or("default"))?;
    let mut system=format!("You are a professional translator. Translate from {source} into {}.\n\n{}\n\nInput is numbered lines. Reply with exactly the same numbers in the same order, one line per number. Preserve the batch's line-break marks. Doubled marks are literal text. Preserve every inline token of the form ⟦XLIFF:n⟧ exactly once, byte-for-byte. No preface, notes or code fences.",request.target_language,purpose["instructions"].as_str().unwrap_or(""));
    if !glossary.is_empty(){system.push_str(&format!("\n\n{}\nThese glossary terms override everything above.",glossary_to_prompt_block(&glossary)));}
    Ok((usable,system,prefs["concurrency"].as_u64().unwrap_or(3) as usize,prefs["per_provider"].as_u64().unwrap_or(1) as usize))
}
#[tauri::command] async fn start_translation(state:State<'_,DesktopState>,app:tauri::AppHandle,request:TranslationRequest)->Result<Value,String>{
    let parent=output_parent(&state,&request)?;let (configs,system,global,per)=job_parameters(&state,&request).await?;let selected=state.selected.lock().unwrap().clone();
    let id=state.jobs.start(request,&selected,&parent,configs,system,global,per,Some(app)).await?;*state.last_job.lock().unwrap()=Some(id.clone());Ok(json!({"jobId":id}))
}
#[tauri::command] fn get_job(state:State<DesktopState>,job_id:String)->Result<jobs::JobSnapshot,String>{state.jobs.snapshot(&job_id)}
#[tauri::command] fn cancel_job(state:State<DesktopState>,job_id:String)->Result<jobs::JobSnapshot,String>{state.jobs.cancel(&job_id)}
#[tauri::command] async fn retry_job(state:State<'_,DesktopState>,app:tauri::AppHandle,job_id:String)->Result<Value,String>{
    let request=state.jobs.retry_request(&job_id)?;let parent=output_parent(&state,&request)?;let (configs,system,global,per)=job_parameters(&state,&request).await?;
    let id=state.jobs.restart(&job_id,&parent,configs,system,global,per,Some(app)).await?;*state.last_job.lock().unwrap()=Some(id.clone());Ok(json!({"jobId":id}))
}
#[tauri::command] fn open_path(state:State<DesktopState>,path:String)->Result<(),String>{let path=PathBuf::from(path).canonicalize().map_err(|e|e.to_string())?;
    if path!=state.store.data_dir()&&!state.jobs.owned_output(&path){return Err("Only the app data directory and owned outputs can be opened.".into());}
    std::process::Command::new("explorer.exe").arg(path).spawn().map_err(|e|e.to_string())?;Ok(())}
#[tauri::command] async fn export_result(state:State<'_,DesktopState>,job_id:String,zip:bool)->Result<Value,String>{
    let _pending=Persistence::new(&state.pending);let manager=state.jobs.clone();
    let (name,bytes)=tokio::task::spawn_blocking(move||manager.export_bytes(&job_id,zip)).await.map_err(|e|e.to_string())??;
    let selected=tokio::task::spawn_blocking(move||rfd::FileDialog::new().set_file_name(name).save_file()).await.map_err(|e|e.to_string())?;
    let Some(path)=selected else{return Ok(json!({"cancelled":true}));};
    // Save As never overwrites an existing owner file; the output service adds a collision suffix.
    tokio::task::spawn_blocking(move||{let actual=jobs::atomic_output(path.parent().ok_or("Choose a destination folder.")?,&path.file_name().ok_or("Choose a filename.")?.to_string_lossy(),&bytes)?;Ok(json!({"path":actual.to_string_lossy(),"bytes":bytes.len()}))}).await.map_err(|e|e.to_string())?}
#[tauri::command] async fn get_preview(state:State<'_,DesktopState>,job_id:Option<String>,input:Option<String>)->Result<Value,String>{
    if let Some(id)=job_id{return state.jobs.preview(&id,input.as_deref());}
    let selected=state.selected.lock().unwrap().clone();let selected=selected.iter().find(|s|input.as_deref().is_none_or(|p|p==s.path)).ok_or("No registered input for preview.")?;
    let path=PathBuf::from(&selected.path);let bytes=jobs::read_bounded(&path,80_000_000)?;Ok(preview_text(&bytes,None))}
fn preview_text(source:&[u8],translated:Option<&[u8]>)->Value{let binary=std::str::from_utf8(source).is_err()||source.contains(&0);let text=if binary{String::new()}else{String::from_utf8_lossy(source).into_owned()};let translated=translated.map(|b|String::from_utf8_lossy(b).into_owned()).unwrap_or_default();let truncated=text.chars().count()>12000||translated.chars().count()>12000;json!({"source":text.chars().take(12000).collect::<String>(),"translated":translated.chars().take(12000).collect::<String>(),"binary":binary,"truncated":truncated})}
#[tauri::command] fn set_unsaved(state:State<DesktopState>,unsaved:bool){state.unsaved.store(unsaved,Ordering::SeqCst);}

#[tauri::command] fn updates_get_preferences(state:State<DesktopState>)->Value{state.updates.get_preferences()}
#[tauri::command] fn updates_set_preferences(state:State<DesktopState>,preferences:Value)->Result<Value,String>{state.updates.set_preferences(preferences)}
#[tauri::command] fn updates_get_state(state:State<DesktopState>)->Value{state.updates.get_state()}
#[tauri::command] async fn updates_check(state:State<'_,DesktopState>,app:tauri::AppHandle)->Result<Value,String>{let result=state.updates.check().await;let _=app.emit("suite-update-state-changed",state.updates.get_state());result}
#[tauri::command] fn updates_download()->Result<Value,String>{Err("Automatic downloading is disabled until a reviewed publisher key and signed native update adapter are configured.".into())}
#[tauri::command] fn updates_cancel(state:State<DesktopState>)->Result<Value,String>{state.updates.cancel()}
#[tauri::command] fn updates_later(state:State<DesktopState>)->Result<Value,String>{state.updates.later()}
#[tauri::command] fn updates_request_install(state:State<DesktopState>,mut work_snapshot:Value)->Result<Value,String>{
    if let Some(object)=work_snapshot.as_object_mut(){let busy=object.get("busy").and_then(Value::as_bool).unwrap_or(true)||state.jobs.active().is_some();let pending=object.get("pendingPersistence").and_then(Value::as_bool).unwrap_or(true)||state.pending.load(Ordering::SeqCst)>0;let unsaved=object.get("unsaved").and_then(Value::as_bool).unwrap_or(true)||state.unsaved.load(Ordering::SeqCst);object.insert("busy".into(),json!(busy));object.insert("pendingPersistence".into(),json!(pending));object.insert("unsaved".into(),json!(unsaved));}state.updates.request_install(work_snapshot)}
#[tauri::command] fn updates_open_official_release(state:State<DesktopState>)->Result<(),String>{std::process::Command::new("explorer.exe").arg(state.updates.official_release_url()).spawn().map_err(|e|e.to_string())?;Ok(())}
#[tauri::command] fn updates_health_ack(state:State<DesktopState>)->Result<Value,String>{let result=state.updates.health_ack()?;state.ui_ready.store(true,Ordering::SeqCst);Ok(result)}

fn main(){
    let data_dir=std::env::var_os("VERSORA_DATA_DIR").map(PathBuf::from).or_else(||std::env::var_os("LOCALAPPDATA").map(|p|PathBuf::from(p).join("Versora"))).expect("Windows LOCALAPPDATA is required");
    let store=match store::Store::new(data_dir.clone()){Ok(value)=>Arc::new(value),Err(error)=>{rfd::MessageDialog::new().set_title("Versora").set_description(format!("資料設定未能讀取；原檔已保留。\n\n{error}")).show();return;}};
    let updates=match suite_updates::UpdateService::new(data_dir,VERSION){Ok(value)=>Arc::new(value),Err(error)=>{rfd::MessageDialog::new().set_title("Versora").set_description(error).show();return;}};
    let initial=std::env::args_os().skip(1).filter(|p|!p.to_string_lossy().starts_with('-')).map(PathBuf::from).collect();
    let selected=register_paths(initial,"files").unwrap_or_default();
    tauri::Builder::default().manage(DesktopState{store,jobs:Arc::new(JobManager::new()),updates,selected:Mutex::new(selected),output_dirs:Mutex::new(Vec::new()),last_job:Mutex::new(None),statuses:Mutex::new(HashMap::new()),proofs:Mutex::new(HashMap::new()),pending:AtomicUsize::new(0),unsaved:AtomicBool::new(false),closing:AtomicBool::new(false),ui_ready:AtomicBool::new(false)})
        .invoke_handler(tauri::generate_handler![get_state,pick_files,clear_selection,choose_output_dir,save_settings,save_provider,delete_provider,probe_provider,test_provider,create_project,load_glossary,save_glossary,save_purpose,import_legacy_data,start_translation,get_job,cancel_job,retry_job,open_path,export_result,get_preview,set_unsaved,updates_get_preferences,updates_set_preferences,updates_get_state,updates_check,updates_download,updates_cancel,updates_later,updates_request_install,updates_open_official_release,updates_health_ack])
        .setup(|app|{let state=app.state::<DesktopState>();if let Some(window)=app.get_webview_window("main"){let _=window.set_theme(Some(if state.store.preferences()["theme"]=="dark"{tauri::Theme::Dark}else{tauri::Theme::Light}));}let handle=app.handle().clone();tauri::async_runtime::spawn(async move{loop{let state=handle.state::<DesktopState>();if state.ui_ready.load(Ordering::SeqCst)&&!state.closing.load(Ordering::SeqCst)&&state.updates.is_check_due(){let _=state.updates.check().await;let _=handle.emit("suite-update-state-changed",state.updates.get_state());}tokio::time::sleep(std::time::Duration::from_secs(1)).await;}});Ok(())})
        .on_window_event(|window,event|match event{
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop{paths,..})=>{let state=window.state::<DesktopState>();if state.jobs.active().is_none(){if let Ok(selected)=register_paths(paths.clone(),"files"){*state.selected.lock().unwrap()=selected;*state.last_job.lock().unwrap()=None;let _=window.emit("selection-changed",json!({}));}}},
            tauri::WindowEvent::CloseRequested{api,..}=>{api.prevent_close();let state=window.state::<DesktopState>();if state.closing.swap(true,Ordering::SeqCst){return;}
                if state.unsaved.load(Ordering::SeqCst)&&rfd::MessageDialog::new().set_title("Versora").set_description("仲有未儲存嘅編輯。離開會放棄呢次編輯。").set_buttons(rfd::MessageButtons::OkCancel).show()!=rfd::MessageDialogResult::Ok{state.closing.store(false,Ordering::SeqCst);return;}
                if let Some(id)=state.jobs.active(){let _=state.jobs.cancel(&id);}let handle=window.app_handle().clone();tauri::async_runtime::spawn(async move{loop{let state=handle.state::<DesktopState>();if state.jobs.active().is_none()&&state.pending.load(Ordering::SeqCst)==0{break;}tokio::time::sleep(std::time::Duration::from_millis(50)).await;}let _=handle.state::<DesktopState>().updates.cancel();handle.exit(0);});},_=>{}
        }).run(tauri::generate_context!()).expect("Native desktop application error");
}
