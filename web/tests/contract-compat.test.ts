import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createFetchTransport, FlashMartClient } from '../src/api/client';
import {
  isApiProblemError,
  type FlashItemListResponse,
  type FlashItemSummary,
  type FlashOrderListResponse,
  type FlashOrderView,
  type OrderMutationRequest,
  type PlaceOrderRequest,
  type ProblemDetails,
} from '../src/api/types';

interface OpenApiSchema {
  type?: string;
  enum?: string[];
  required?: string[];
  properties?: Record<string, { $ref?: string; type?: string; nullable?: boolean }>;
}

interface OpenApiSpec {
  paths: Record<
    string,
    Record<
      string,
      {
        parameters?: Array<{ in: string; name: string; required?: boolean }>;
        requestBody?: { content?: { 'application/json'?: { schema?: { $ref?: string } } } };
      }
    >
  >;
  components: {
    schemas: Record<string, OpenApiSchema>;
  };
}

function loadOpenApiSpec(): OpenApiSpec {
  const path = resolve(__dirname, '../../contracts/openapi.json');
  return JSON.parse(readFileSync(path, 'utf8')) as OpenApiSpec;
}

function schemaProps(spec: OpenApiSpec, schemaName: string): string[] {
  const schema = spec.components.schemas[schemaName];
  expect(schema, `OpenAPI schema ${schemaName} must exist`).toBeDefined();
  return Object.keys(schema.properties ?? {}).sort();
}

describe('OpenAPI 契约与 Fetch 客户端兼容性校验', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('contracts/openapi.json 的路径、请求头与结构字段与前端 types/client 严格对齐', () => {
    const spec = loadOpenApiSpec();

    const expectedRoutes: Array<[string, string, string[]]> = [
      ['/api/health', 'get', []],
      ['/api/flash-items', 'get', []],
      ['/api/flash-items/{item_id}', 'get', []],
      ['/api/orders', 'get', ['X-Buyer-Id']],
      ['/api/orders', 'post', ['X-Buyer-Id', 'Idempotency-Key']],
      ['/api/orders/{order_id}', 'get', ['X-Buyer-Id']],
      ['/api/orders/{order_id}/pay', 'post', ['X-Buyer-Id', 'Idempotency-Key']],
      ['/api/orders/{order_id}/cancel', 'post', ['X-Buyer-Id', 'Idempotency-Key']],
    ];

    for (const [path, method, requiredHeaders] of expectedRoutes) {
      const op = spec.paths[path]?.[method];
      expect(op, `Missing ${method.toUpperCase()} ${path} in openapi.json`).toBeDefined();
      const actualHeaders = (op?.parameters ?? [])
        .filter((p) => p.in === 'header' && p.required)
        .map((p) => p.name)
        .sort();
      expect(actualHeaders).toEqual([...requiredHeaders].sort());
    }

    expect(spec.components.schemas.ItemKind.enum).toEqual(['digital', 'physical']);
    expect(spec.components.schemas.WindowStatus.enum).toEqual(['upcoming', 'active', 'ended']);
    expect(spec.components.schemas.OrderLifecycleStatus.enum).toEqual([
      'pending_payment',
      'paid',
      'cancelled',
      'expired',
    ]);

    const itemFields: Array<keyof FlashItemSummary> = [
      'id',
      'title',
      'description',
      'item_kind',
      'price_cents',
      'currency',
      'total_stock',
      'available_stock',
      'per_user_limit',
      'sale_starts_at',
      'sale_ends_at',
      'window_status',
      'server_time',
    ];
    expect(schemaProps(spec, 'FlashItemSummary')).toEqual([...itemFields].sort());
    expect(schemaProps(spec, 'FlashItemDetailView')).toEqual([...itemFields].sort());

    const itemListFields: Array<keyof FlashItemListResponse> = ['items', 'server_time'];
    expect(schemaProps(spec, 'FlashItemListResponse')).toEqual([...itemListFields].sort());

    const orderFields: Array<keyof FlashOrderView> = [
      'id',
      'buyer_id',
      'item_id',
      'item_title',
      'item_kind',
      'quantity',
      'unit_price_cents',
      'total_amount_cents',
      'currency',
      'status',
      'version',
      'payment_deadline',
      'created_at',
      'paid_at',
      'cancelled_at',
      'expired_at',
      'fulfillment_hint',
    ];
    for (const viewName of [
      'FlashOrderView',
      'FlashOrderSummaryView',
      'FlashOrderDetailView',
      'PaidOrderView',
      'CancelledOrderView',
    ]) {
      expect(schemaProps(spec, viewName)).toEqual([...orderFields].sort());
    }

    const orderListFields: Array<keyof FlashOrderListResponse> = ['orders', 'server_time'];
    expect(schemaProps(spec, 'FlashOrderListResponse')).toEqual([...orderListFields].sort());

    const placeReqFields: Array<keyof PlaceOrderRequest> = ['item_id', 'quantity'];
    expect(schemaProps(spec, 'PlaceOrderRequest')).toEqual([...placeReqFields].sort());

    const mutReqFields: Array<keyof OrderMutationRequest> = ['version'];
    expect(schemaProps(spec, 'PayOrderRequest')).toEqual([...mutReqFields].sort());
    expect(schemaProps(spec, 'CancelOrderRequest')).toEqual([...mutReqFields].sort());

    const problemFields: Array<keyof ProblemDetails> = [
      'type',
      'title',
      'status',
      'detail',
      'instance',
      'code',
      'errors',
    ];
    expect(schemaProps(spec, 'ProblemDetails')).toEqual([...problemFields].sort());
  });

  it('createFetchTransport 正确透传鉴权幂等头并解析 RFC 9457 问题详情与非 JSON 故障响应', async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            id: 'ord-1',
            buyer_id: 'buyer-live',
            item_id: '11111111-1111-4111-8111-111111111101',
            item_title: 'Test',
            item_kind: 'digital',
            quantity: 1,
            unit_price_cents: 9900,
            total_amount_cents: 9900,
            currency: 'CNY',
            status: 'pending_payment',
            version: 1,
            payment_deadline: '2026-11-21T12:15:00Z',
            created_at: '2026-11-21T12:00:00Z',
            paid_at: null,
            cancelled_at: null,
            expired_at: null,
            fulfillment_hint: null,
          }),
          { status: 201, headers: { 'Content-Type': 'application/json' } },
        ),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            type: 'urn:flashmart:problem:insufficient_stock',
            title: '库存不足',
            status: 409,
            detail: '当前商品可售库存为 0。',
            instance: '/api/orders',
            code: 'insufficient_stock',
          }),
          { status: 409, headers: { 'Content-Type': 'application/problem+json' } },
        ),
      )
      .mockResolvedValueOnce(new Response('Bad Gateway', { status: 502 }));

    vi.stubGlobal('fetch', fetchMock);
    const client = new FlashMartClient(createFetchTransport('http://127.0.0.1:8080'));

    const created = await client.placeOrder(
      'buyer-live',
      { item_id: '11111111-1111-4111-8111-111111111101', quantity: 1 },
      'idem-live-1',
    );
    expect(created.id).toBe('ord-1');
    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      'http://127.0.0.1:8080/api/orders',
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({
          'X-Buyer-Id': 'buyer-live',
          'Idempotency-Key': 'idem-live-1',
          'Content-Type': 'application/json',
        }),
      }),
    );

    try {
      await client.placeOrder(
        'buyer-live',
        { item_id: '11111111-1111-4111-8111-111111111101', quantity: 1 },
        'idem-live-2',
      );
      expect.unreachable('should throw ApiProblemError for 409');
    } catch (error) {
      expect(isApiProblemError(error)).toBe(true);
      if (isApiProblemError(error)) {
        expect(error.httpStatus).toBe(409);
        expect(error.code).toBe('insufficient_stock');
      }
    }

    try {
      await client.health();
      expect.unreachable('should throw ApiProblemError for 502');
    } catch (error) {
      expect(isApiProblemError(error)).toBe(true);
      if (isApiProblemError(error)) {
        expect(error.httpStatus).toBe(502);
        expect(error.code).toBe('unexpected_error');
      }
    }
  });
});
