import {
  ApiProblemError,
  type FlashItemListResponse,
  type FlashItemSummary,
  type FlashOrderListResponse,
  type FlashOrderView,
  type OrderMutationRequest,
  type PlaceOrderRequest,
  type ProblemDetails,
} from './types';

export interface TransportRequest {
  method: 'GET' | 'POST';
  path: string;
  headers?: Record<string, string>;
  body?: unknown;
  signal?: AbortSignal;
}

export interface Transport {
  request<T>(req: TransportRequest): Promise<T>;
}

export function createFetchTransport(baseUrl = ''): Transport {
  return {
    async request<T>(req: TransportRequest): Promise<T> {
      const headers: Record<string, string> = {
        Accept: 'application/json, application/problem+json',
        ...(req.headers ?? {}),
      };
      if (req.body !== undefined) {
        headers['Content-Type'] = 'application/json';
      }

      let response: Response;
      try {
        response = await fetch(`${baseUrl}${req.path}`, {
          method: req.method,
          headers,
          body: req.body !== undefined ? JSON.stringify(req.body) : undefined,
          signal: req.signal,
        });
      } catch (error) {
        if (error instanceof DOMException && error.name === 'AbortError') {
          throw error;
        }
        throw new ApiProblemError({
          type: 'urn:flashmart:problem:network_error',
          title: '网络连接不可用',
          status: 0,
          detail: '无法连接服务端，请检查网络或服务运行状态后重试。',
          instance: req.path,
          code: 'network_error',
        });
      }

      const text = await response.text();
      const parsed = text ? (JSON.parse(text) as unknown) : null;

      if (!response.ok) {
        const problem = normalizeProblemDetails(parsed, response.status, req.path);
        throw new ApiProblemError(problem);
      }

      return parsed as T;
    },
  };
}

function normalizeProblemDetails(
  payload: unknown,
  status: number,
  path: string,
): ProblemDetails {
  if (
    payload &&
    typeof payload === 'object' &&
    'title' in payload &&
    'detail' in payload &&
    'code' in payload
  ) {
    const record = payload as Partial<ProblemDetails>;
    return {
      type: record.type ?? `urn:flashmart:problem:${record.code ?? 'unknown'}`,
      title: String(record.title),
      status: typeof record.status === 'number' ? record.status : status,
      detail: String(record.detail),
      instance: record.instance ?? path,
      code: String(record.code),
      errors: Array.isArray(record.errors) ? record.errors : undefined,
    };
  }
  return {
    type: 'urn:flashmart:problem:unexpected_error',
    title: '请求处理失败',
    status,
    detail: `服务端返回了非预期响应 (${status})`,
    instance: path,
    code: 'unexpected_error',
  };
}

export class FlashMartClient {
  private readonly transport: Transport;

  constructor(transport: Transport) {
    this.transport = transport;
  }

  health(signal?: AbortSignal): Promise<{ status: string }> {
    return this.transport.request({ method: 'GET', path: '/api/health', signal });
  }

  listFlashItems(signal?: AbortSignal): Promise<FlashItemListResponse> {
    return this.transport.request({
      method: 'GET',
      path: '/api/flash-items',
      signal,
    });
  }

  getFlashItem(itemId: string, signal?: AbortSignal): Promise<FlashItemSummary> {
    return this.transport.request({
      method: 'GET',
      path: `/api/flash-items/${encodeURIComponent(itemId)}`,
      signal,
    });
  }

  placeOrder(
    buyerId: string,
    input: PlaceOrderRequest,
    idempotencyKey: string,
    signal?: AbortSignal,
  ): Promise<FlashOrderView> {
    return this.transport.request({
      method: 'POST',
      path: '/api/orders',
      headers: {
        'X-Buyer-Id': buyerId,
        'Idempotency-Key': idempotencyKey,
      },
      body: input,
      signal,
    });
  }

  listOrders(buyerId: string, signal?: AbortSignal): Promise<FlashOrderListResponse> {
    return this.transport.request({
      method: 'GET',
      path: '/api/orders',
      headers: {
        'X-Buyer-Id': buyerId,
      },
      signal,
    });
  }

  getOrder(
    buyerId: string,
    orderId: string,
    signal?: AbortSignal,
  ): Promise<FlashOrderView> {
    return this.transport.request({
      method: 'GET',
      path: `/api/orders/${encodeURIComponent(orderId)}`,
      headers: {
        'X-Buyer-Id': buyerId,
      },
      signal,
    });
  }

  payOrder(
    buyerId: string,
    orderId: string,
    body: OrderMutationRequest,
    idempotencyKey: string,
    signal?: AbortSignal,
  ): Promise<FlashOrderView> {
    return this.transport.request({
      method: 'POST',
      path: `/api/orders/${encodeURIComponent(orderId)}/pay`,
      headers: {
        'X-Buyer-Id': buyerId,
        'Idempotency-Key': idempotencyKey,
      },
      body,
      signal,
    });
  }

  cancelOrder(
    buyerId: string,
    orderId: string,
    body: OrderMutationRequest,
    idempotencyKey: string,
    signal?: AbortSignal,
  ): Promise<FlashOrderView> {
    return this.transport.request({
      method: 'POST',
      path: `/api/orders/${encodeURIComponent(orderId)}/cancel`,
      headers: {
        'X-Buyer-Id': buyerId,
        'Idempotency-Key': idempotencyKey,
      },
      body,
      signal,
    });
  }
}
