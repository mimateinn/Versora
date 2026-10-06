import assert from 'node:assert/strict';
import test from 'node:test';
import {progressState} from '../../ui/progress-state.js';

test('startup and pending native commands have no invented percentage', () => {
  assert.equal(progressState({initializing:true}).kind, 'indeterminate');
  assert.equal(progressState({busy:true}).kind, 'indeterminate');
  assert.deepEqual(progressState(), {kind:'idle'});
});

test('a single file reports real chunk counts, including cancellation', () => {
  const job={status:'running',total:1,done:0,chunksTotal:8,chunksDone:3};
  assert.deepEqual(progressState({job}), {kind:'determinate',unit:'chunks',done:3,total:8,label:'run.chunks'});
  assert.equal(progressState({job:{...job,status:'cancelling'}}).label, 'run.cancelling');
  assert.equal(progressState({job:{...job,status:'cancelling'}}).done, 3);
});

test('a batch uses completed files and takes priority over background work', () => {
  const job={status:'running',total:5,done:2,chunksTotal:20,chunksDone:9};
  assert.deepEqual(progressState({job,busy:true,update:{status:'downloading',totalBytes:10,bytesReceived:8}}),
    {kind:'determinate',unit:'files',done:2,total:5,label:'run.files'});
});

test('finished, failed, and stopped jobs release the global strip', () => {
  for(const status of ['done','error','stopped'])
    assert.deepEqual(progressState({job:{status,total:1,done:1}}), {kind:'idle'});
});

test('a command finishing a job stays busy until its native response returns', () => {
  assert.equal(progressState({busy:true,job:{status:'stopped'}}).kind, 'indeterminate');
  assert.equal(progressState({busy:false,job:{status:'stopped'}}).kind, 'idle');
});

test('update checks and downloads use native byte counts or an indeterminate cue', () => {
  assert.equal(progressState({update:{status:'checking'}}).kind, 'indeterminate');
  assert.equal(progressState({update:{status:'downloading',totalBytes:null}}).kind, 'indeterminate');
  assert.deepEqual(progressState({update:{status:'downloading',totalBytes:200,bytesReceived:80}}),
    {kind:'determinate',unit:'bytes',done:80,total:200,label:'updates.downloading_p'});
  for(const status of ['idle','ready','failed','cancelled'])
    assert.equal(progressState({update:{status}}).kind, 'idle');
});

test('native counters are finite, nonnegative, and clamped to the total', () => {
  for(const done of [-1,NaN,Infinity,null,undefined,'invalid'])
    assert.equal(progressState({job:{status:'running',total:3,done}}).done, 0);
  assert.equal(progressState({job:{status:'running',total:3,done:100}}).done, 3);
  assert.equal(progressState({job:{status:'running',total:1,chunksTotal:4,chunksDone:9}}).done, 4);
  assert.equal(progressState({update:{status:'downloading',totalBytes:10,bytesReceived:20}}).done, 10);
});
