// The orders service, in Thrift. Written to exercise every field the
// plugin extracts: namespaces per language, an include, a typedef, an
// enumeration, structs, an exception, and a service that extends another
// with a oneway method and a throwing one.

namespace java com.example.orders
namespace py example.orders
namespace go example.orders

include "common.thrift"

typedef i64 Timestamp
typedef string OrderId

enum State {
  DRAFT = 0,
  PLACED = 1,
  SHIPPED = 2,
  CANCELLED = 3,
}

struct Line {
  1: required string sku,
  2: required i32 quantity,
  3: optional i64 unitPricePennies,
}

struct Order {
  1: required OrderId id,
  2: required string customerId,
  3: required list<Line> lines,
  4: optional string note,
  // Neither required nor optional: the Java binding writes it always, the
  // Python one omits it when unset, and nobody notices until they
  // disagree across a wire.
  5: State state,
  6: Timestamp placedAt,
}

union Payment {
  1: string cardLastFour,
  2: string accountId,
}

exception NotFound {
  1: required string id,
  2: optional string message,
}

exception Invalid {
  1: required string reason,
}

service Base {
  string ping(),
}

service Orders extends Base {
  Order get(1: required string id) throws (1: NotFound missing),
  list<Order> list(1: string customerId, 2: i32 limit),
  void cancel(1: required string id) throws (1: NotFound missing, 2: Invalid bad),
  oneway void audit(1: string id),
}
