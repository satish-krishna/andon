// Use OTel SDK to send a metric over gRPC, mirroring what Claude Code does.
const { MeterProvider, PeriodicExportingMetricReader } = require('@opentelemetry/sdk-metrics');
const { OTLPMetricExporter } = require('@opentelemetry/exporter-metrics-otlp-grpc');
const { resourceFromAttributes } = require('@opentelemetry/resources');

const exporter = new OTLPMetricExporter({
  url: 'http://localhost:4317',
});

const reader = new PeriodicExportingMetricReader({
  exporter,
  exportIntervalMillis: 1000,
});

const provider = new MeterProvider({
  resource: resourceFromAttributes({
    'service.name': 'claude-code',
    'service.version': 'smoke-grpc',
    'session.id': 'grpc-smoke-001',
    'host.arch': 'amd64',
    'os.type': 'windows',
  }),
  readers: [reader],
});

const meter = provider.getMeter('claude-code-smoke');
const cost = meter.createCounter('claude_code.cost.usage');
const tokens = meter.createCounter('claude_code.token.usage');

cost.add(0.1234, { model: 'claude-sonnet-4-6' });
tokens.add(500, { type: 'input', model: 'claude-sonnet-4-6' });
tokens.add(123, { type: 'output', model: 'claude-sonnet-4-6' });

console.log('emitting... will wait 3s for export then force-flush');
setTimeout(async () => {
  await provider.forceFlush();
  await provider.shutdown();
  console.log('done');
  process.exit(0);
}, 3000);
