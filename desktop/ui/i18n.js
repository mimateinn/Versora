export const languages = ['zh-Hant','zh-Hans','en','ja','ko','es','fr','de','pt','vi','th','id'];
export const nativeLanguageNames = {'zh-Hant':'繁體中文','zh-Hans':'简体中文',en:'English',ja:'日本語',ko:'한국어',es:'Español',fr:'Français',de:'Deutsch',pt:'Português',vi:'Tiếng Việt',th:'ไทย',id:'Bahasa Indonesia'};
const catalogs = new Map();
const demoCompletion = {'zh-Hant':'Demo 測試完成','zh-Hans':'Demo 测试完成',en:'Demo completed',ja:'Demo テスト完了',ko:'Demo 테스트 완료',es:'Prueba Demo completada',fr:'Test Demo terminé',de:'Demo-Test abgeschlossen',pt:'Teste Demo concluído',vi:'Đã hoàn tất thử nghiệm Demo',th:'การทดสอบ Demo เสร็จสิ้น',id:'Uji Demo selesai'};
let current = 'en';
const desktopCopy = {
  en: ['Import existing settings','Windows protects saved API keys for your user account. Full keys are never returned to this screen.','Uses this computer’s language when available.','Output folder','Choose folder','Uses the app’s output folder when empty.','Save result as…','Application data','Re-check local programs','Demo: offline test only, no real translation','Selected files','Source files are kept unchanged.','Reduce motion','Save settings','No files selected','Translation requires a configured translator.','A transport test sends a real request and may use your provider quota.'],
  'zh-Hant': ['匯入現有設定','Windows 會為你的使用者帳戶保護已儲存的 API 金鑰。完整金鑰不會傳回此畫面。','可用時跟隨這部電腦的語言。','輸出資料夾','選擇資料夾','留空時使用程式的輸出資料夾。','另存結果…','應用程式資料','重新檢查本機程式','Demo：離線測試，並非真正翻譯','已選檔案','來源檔案保持原樣。','減少動態效果','儲存設定','未選擇檔案','翻譯需要已設定的翻譯器。','連線測試會發出真正請求，可能使用供應商額度。'],
  'zh-Hans': ['导入现有设置','Windows 会为你的用户账户保护已保存的 API 密钥。完整密钥不会返回此界面。','可用时跟随这台电脑的语言。','输出文件夹','选择文件夹','留空时使用程序的输出文件夹。','另存结果…','应用程序数据','重新检查本地程序','Demo：离线测试，并非真正翻译','已选文件','源文件保持原样。','减少动态效果','保存设置','未选择文件','翻译需要已设置的翻译器。','连接测试会发出真实请求，可能使用供应商额度。'],
  ja: ['既存の設定を取り込む','APIキーはWindowsのユーザーアカウントで保護されます。完全なキーは画面に返されません。','可能な場合はこのコンピューターの言語を使用します。','出力フォルダー','フォルダーを選択','空欄の場合はアプリの出力フォルダーを使います。','結果を名前を付けて保存…','アプリのデータ','ローカルプログラムを再確認','Demo：オフラインのテストのみ。実際の翻訳ではありません。','選択したファイル','元のファイルは変更されません。','動きを減らす','設定を保存','ファイル未選択','設定済みの翻訳サービスが必要です。','接続テストは実際のリクエストを送信し、利用枠を消費する場合があります。'],
  ko: ['기존 설정 가져오기','API 키는 Windows 사용자 계정으로 보호됩니다. 전체 키는 화면에 반환되지 않습니다.','가능하면 이 컴퓨터의 언어를 사용합니다.','출력 폴더','폴더 선택','비워 두면 앱의 출력 폴더를 사용합니다.','결과를 다른 이름으로 저장…','앱 데이터','로컬 프로그램 다시 확인','Demo: 오프라인 테스트이며 실제 번역이 아닙니다.','선택한 파일','원본 파일은 변경하지 않습니다.','동작 줄이기','설정 저장','선택한 파일 없음','설정된 번역 서비스가 필요합니다.','연결 테스트는 실제 요청을 보내며 사용량이 차감될 수 있습니다.'],
  es: ['Importar ajustes existentes','Windows protege las claves API de tu cuenta. Nunca se devuelven claves completas a esta pantalla.','Usa el idioma del equipo si está disponible.','Carpeta de salida','Elegir carpeta','Vacío: usa la carpeta de salida de la aplicación.','Guardar resultado como…','Datos de la aplicación','Comprobar programas locales','Demo: prueba sin conexión, sin traducción real','Archivos seleccionados','Los originales no se modifican.','Reducir movimiento','Guardar ajustes','Ningún archivo seleccionado','Configura un traductor para traducir.','La prueba envía una solicitud real y puede consumir cuota.'],
  fr: ['Importer les réglages existants','Windows protège les clés API de votre compte. Les clés complètes ne sont jamais renvoyées à cet écran.','Utilise la langue de cet ordinateur si disponible.','Dossier de sortie','Choisir un dossier','Vide : utilise le dossier de sortie de l’application.','Enregistrer le résultat sous…','Données de l’application','Vérifier les programmes locaux','Démo : test hors ligne, sans traduction réelle','Fichiers sélectionnés','Les originaux restent inchangés.','Réduire les animations','Enregistrer les réglages','Aucun fichier sélectionné','Configurez un traducteur pour traduire.','Le test envoie une requête réelle et peut consommer votre quota.'],
  de: ['Vorhandene Einstellungen importieren','Windows schützt API-Schlüssel für Ihr Benutzerkonto. Vollständige Schlüssel werden nie an diese Ansicht zurückgegeben.','Verwendet nach Möglichkeit die Sprache dieses Computers.','Ausgabeordner','Ordner wählen','Leer: Ausgabeordner der Anwendung verwenden.','Ergebnis speichern unter…','Anwendungsdaten','Lokale Programme prüfen','Demo: Offline-Test, keine echte Übersetzung','Ausgewählte Dateien','Originaldateien bleiben unverändert.','Bewegung reduzieren','Einstellungen speichern','Keine Dateien ausgewählt','Richten Sie einen Übersetzer ein.','Der Test sendet eine echte Anfrage und kann Kontingent verbrauchen.'],
  pt: ['Importar definições existentes','O Windows protege as chaves API da sua conta. As chaves completas nunca regressam a este ecrã.','Usa o idioma deste computador quando disponível.','Pasta de saída','Escolher pasta','Vazio: usa a pasta de saída da aplicação.','Guardar resultado como…','Dados da aplicação','Verificar programas locais','Demo: teste offline, sem tradução real','Ficheiros selecionados','Os originais permanecem inalterados.','Reduzir movimento','Guardar definições','Nenhum ficheiro selecionado','Configure um tradutor para traduzir.','O teste envia um pedido real e pode consumir quota.'],
  vi: ['Nhập cài đặt hiện có','Windows bảo vệ khóa API cho tài khoản của bạn. Khóa đầy đủ không bao giờ được trả về màn hình này.','Dùng ngôn ngữ máy tính khi có thể.','Thư mục đầu ra','Chọn thư mục','Để trống để dùng thư mục đầu ra của ứng dụng.','Lưu kết quả thành…','Dữ liệu ứng dụng','Kiểm tra lại chương trình cục bộ','Demo: thử ngoại tuyến, không dịch thật','Tệp đã chọn','Tệp gốc được giữ nguyên.','Giảm chuyển động','Lưu cài đặt','Chưa chọn tệp','Cần cấu hình trình dịch để dịch.','Kiểm tra kết nối gửi yêu cầu thật và có thể dùng hạn mức.'],
  th: ['นำเข้าการตั้งค่าเดิม','Windows ปกป้องคีย์ API สำหรับบัญชีผู้ใช้ของคุณ โดยจะไม่ส่งคีย์ทั้งหมดกลับมายังหน้าจอนี้','ใช้ภาษาของคอมพิวเตอร์นี้เมื่อมีให้เลือก','โฟลเดอร์ผลลัพธ์','เลือกโฟลเดอร์','เว้นว่างเพื่อใช้โฟลเดอร์ผลลัพธ์ของแอป','บันทึกผลลัพธ์เป็น…','ข้อมูลแอป','ตรวจสอบโปรแกรมในเครื่องอีกครั้ง','Demo: ทดสอบออฟไลน์ ไม่ใช่การแปลจริง','ไฟล์ที่เลือก','ไฟล์ต้นฉบับจะไม่เปลี่ยนแปลง','ลดการเคลื่อนไหว','บันทึกการตั้งค่า','ยังไม่ได้เลือกไฟล์','ต้องตั้งค่าผู้แปลก่อนแปล','การทดสอบจะส่งคำขอจริงและอาจใช้โควตาของผู้ให้บริการ'],
  id: ['Impor pengaturan yang ada','Windows melindungi kunci API akun Anda. Kunci lengkap tidak pernah dikirim kembali ke layar ini.','Gunakan bahasa komputer ini jika tersedia.','Folder keluaran','Pilih folder','Kosong: gunakan folder keluaran aplikasi.','Simpan hasil sebagai…','Data aplikasi','Periksa program lokal lagi','Demo: uji offline, bukan terjemahan nyata','Berkas dipilih','Berkas asli tidak diubah.','Kurangi gerakan','Simpan pengaturan','Belum memilih berkas','Konfigurasikan penerjemah untuk menerjemahkan.','Uji koneksi mengirim permintaan nyata dan mungkin menggunakan kuota.'],
};
const desktopKeys = ['desktop.import','desktop.keys','desktop.os_language','desktop.output','desktop.choose_folder','desktop.output_hint','desktop.save_as','desktop.data','desktop.recheck','desktop.demo','desktop.selected','desktop.originals','desktop.motion','desktop.save_settings','desktop.no_files','desktop.no_provider','desktop.transport_note'];
export function systemLanguage() {
  for (const raw of navigator.languages || [navigator.language]) {
    const code = raw.replace('_','-');
    if (/^zh-(TW|HK|MO|Hant)/i.test(code)) return 'zh-Hant';
    if (/^zh/i.test(code)) return 'zh-Hans';
    const match = languages.find(value => value.toLowerCase() === code.toLowerCase()) || languages.find(value => value === code.split('-')[0]);
    if (match) return match;
  }
  return 'en';
}
async function catalog(code) {
  if (!catalogs.has(code)) {
    const response = await fetch(`./locales/${code}.json`);
    if (!response.ok) throw new Error(`Cannot load ${code} interface catalog`);
    catalogs.set(code, await response.json());
  }
  return catalogs.get(code);
}
export async function setLanguage(code) {
  current = languages.includes(code) ? code : 'en';
  await Promise.all([catalog('en'), catalog(current)]);
  document.documentElement.lang = current;
}
export function t(key, variables = {}) {
  if (key === 'desktop.demo_completed') return demoCompletion[current] || demoCompletion.en;
  const extra = desktopKeys.indexOf(key);
  let value = updateTranslation(key,current) ?? (extra >= 0 ? (desktopCopy[current] || desktopCopy.en)[extra] : catalogs.get(current)?.[key] ?? catalogs.get('en')?.[key] ?? key);
  // Original Python copy mentioned plain .env storage. Native secrets are DPAPI protected.
  if (['keys.local_only','keys.local_plain'].includes(key)) value = t('desktop.keys');
  if (key === 'card.lang_os') value = t('desktop.os_language');
  if (key === 'main.folder_hint') value = t('desktop.choose_folder');
  if (key === 'keys.paste_api') value = current === 'en' ? 'Paste API key' : catalogs.get(current)?.['svc.paste']?.replace('{name}', 'API') || 'API key';
  return String(value).replace(/\{([a-zA-Z_][\w]*)\}/g, (match,name) => variables[name] ?? match);
}
export function translateDocument(root = document) {
  root.querySelectorAll('[data-i18n]').forEach(node => { node.textContent = t(node.dataset.i18n); });
}
import {updateTranslation} from './updates-i18n.js';
