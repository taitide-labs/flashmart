export type ItemKind = 'digital' | 'physical';
export type WindowStatus = 'upcoming' | 'active' | 'ended';
export type OrderStatus = 'pending_payment' | 'paid' | 'cancelled' | 'expired';

export interface FlashItemSummary {
  id: string;
  title: string;
  description: string;
  item_kind: ItemKind;
  price_cents: number;
  currency: 'CNY';
  total_stock: number;
  available_stock: number;
  per_user_limit: number;
  sale_starts_at: string;
  sale_ends_at: string;
  window_status: WindowStatus;
  server_time: string;
}

export interface FlashItemListResponse {
  items: FlashItemSummary[];
  server_time: string;
}

export interface FlashOrderView {
  id: string;
  buyer_id: string;
  item_id: string;
  item_title: string;
  item_kind: ItemKind;
  quantity: number;
  unit_price_cents: number;
  total_amount_cents: number;
  currency: 'CNY';
  status: OrderStatus;
  version: number;
  payment_deadline: string;
  created_at: string;
  paid_at: string | null;
  cancelled_at: string | null;
  expired_at: string | null;
  fulfillment_hint: string | null;
}

export interface FlashOrderListResponse {
  orders: FlashOrderView[];
  server_time: string;
}

export interface PlaceOrderRequest {
  item_id: string;
  quantity: number;
}

export interface OrderMutationRequest {
  version: number;
}

export interface FieldValidationError {
  field: string;
  message: string;
}

export interface ProblemDetails {
  type: string;
  title: string;
  status: number;
  detail: string;
  instance: string;
  code: string;
  errors?: FieldValidationError[];
}

export class ApiProblemError extends Error {
  readonly problem: ProblemDetails;
  readonly httpStatus: number;
  readonly code: string;

  constructor(problem: ProblemDetails) {
    super(problem.detail || problem.title);
    this.name = 'ApiProblemError';
    this.problem = problem;
    this.httpStatus = problem.status;
    this.code = problem.code;
  }
}

export function isApiProblemError(error: unknown): error is ApiProblemError {
  return error instanceof ApiProblemError;
}
