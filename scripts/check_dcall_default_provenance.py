"""Compile positive and negative API probes against the default DATUM library."""
from pathlib import Path
import os
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
DATUM = ROOT / 'DATUM'
env = os.environ.copy()
build = subprocess.run(['cargo', 'build', '--locked', '--lib'], cwd=DATUM)
if build.returncode:
    raise SystemExit(build.returncode)

positive = '''
use datum::dcall::delivery::{dispatch_validated_emit, SourceExecutionContext};
use datum::dcode::invoke::{GovernedDCodeInvocationResult, invoke_dcode_governed_capturing_emits};
fn dispatch(c: &SourceExecutionContext, r: &GovernedDCodeInvocationResult) { let _ = dispatch_validated_emit(c, r, 0); }
fn invoke(s: &datum::dcode::module_store::DCodeModuleStore, a: &datum::dcode::authorization::DCodeInvocationAuthorizationV1, d: &datum::dcode::dserv_artifact::CanonicalDServArtifactV1, root: &std::path::Path) { let _ = invoke_dcode_governed_capturing_emits(s, a, d, root, "in", b"input"); }
'''
negative = {
    'generic-result': ('fn f(c: &datum::dcall::delivery::SourceExecutionContext, r: &datum::dcode::invoke::DCodeInvocationResult) { datum::dcall::delivery::dispatch_validated_emit(c, r, 0); }', 'mismatched types'),
    'executor-to-dispatch': ('fn f(c: &datum::dcall::delivery::SourceExecutionContext, r: &dyn datum::dcode::invoke::DCodeExecutor) { datum::dcall::delivery::dispatch_validated_emit(c, r, 0); }', 'mismatched types'),
    'private-governed-constructor': ('fn f(r: datum::dcode::invoke::DCodeInvocationResult) { let _ = datum::dcode::invoke::GovernedDCodeInvocationResult { inner: r }; }', 'private'),
    'no-generic-conversion': ('fn f(r: datum::dcode::invoke::DCodeInvocationResult) -> datum::dcode::invoke::GovernedDCodeInvocationResult { r.into() }', 'trait bound'),
    'test-invoke-hidden': ('use datum::dcode::invoke::invoke_dcode_governed_capturing_emits_for_test;', 'unresolved import'),
    'test-dispatch-hidden': ('use datum::dcall::delivery::dispatch_validated_emit_for_test;', 'unresolved import'),
    'worker-override': ('fn f(c: &datum::dcall::delivery::SourceExecutionContext, r: &datum::dcode::invoke::GovernedDCodeInvocationResult) { datum::dcall::delivery::dispatch_validated_emit(c, r, 0, std::path::Path::new("fake-worker")); }', 'takes 3 arguments'),
    'node-override': ('fn f(c: &datum::dcall::delivery::SourceExecutionContext, r: &datum::dcode::invoke::GovernedDCodeInvocationResult) { datum::dcall::delivery::dispatch_validated_emit(c, r, 0, "caller-node"); }', 'takes 3 arguments'),
    'destination-bypass-hidden': ('use datum::dcall::delivery::execute_destination_delivery;', 'private'),
}
with tempfile.TemporaryDirectory(prefix='datum-dcall-default-api-') as temporary:
    temp = Path(temporary)
    for name, (source, expected) in {'positive': (positive, None), **negative}.items():
        path = temp / (name + '.rs')
        path.write_text(source, encoding='utf-8')
        command = ['rustc', '--edition=2021', '--crate-name', 'probe', '--crate-type', 'lib', str(path),
                   '--extern', 'datum=' + str(DATUM / 'target/debug/libdatum.rlib'),
                   '-L', 'dependency=' + str(DATUM / 'target/debug/deps'), '--out-dir', str(temp)]
        result = subprocess.run(command, capture_output=True, text=True)
        if expected is None:
            assert result.returncode == 0, result.stderr
        else:
            assert result.returncode != 0 and expected in result.stderr, name + ': ' + result.stderr
        print(name + ': PASS', flush=True)
print('default provenance: 1 positive + 9 negative compiler probes PASS')
