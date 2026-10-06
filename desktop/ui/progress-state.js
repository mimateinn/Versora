/* One titlebar strip reflects real work, independently of the current page. */
const count = value => Number.isFinite(Number(value)) ? Math.max(0, Math.trunc(Number(value))) : 0;

export function progressState({initializing = false, busy = false, job = null, update = null} = {}) {
  if (['running', 'cancelling'].includes(job?.status)) {
    const chunks = count(job.chunksTotal);
    const unit = count(job.total) <= 1 && chunks > 0 ? 'chunks' : 'files';
    const total = unit === 'chunks' ? chunks : Math.max(1, count(job.total));
    const done = Math.min(total, count(unit === 'chunks' ? job.chunksDone : job.done));
    return {kind: 'determinate', unit, done, total, label: job.status === 'cancelling' ? 'run.cancelling' : unit === 'chunks' ? 'run.chunks' : 'run.files'};
  }
  if (initializing || busy) return {kind: 'indeterminate', label: initializing ? 'updates.loading' : 'state.running'};
  if (update?.status === 'downloading') {
    const total = count(update.totalBytes);
    return total > 0
      ? {kind: 'determinate', unit: 'bytes', done: Math.min(total, count(update.bytesReceived)), total, label: 'updates.downloading_p'}
      : {kind: 'indeterminate', label: 'updates.downloading'};
  }
  if (update?.status === 'checking' || update?.active) return {kind: 'indeterminate', label: 'updates.checking'};
  return {kind: 'idle'};
}
