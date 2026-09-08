package com.example.streaming

import scala.collection.immutable.Queue
import scala.concurrent.{ExecutionContext, Future}
import scala.util.{Failure, Success, Try}

/** A replayable event stream: events are appended with a monotonic offset,
  * consumers read from an offset, and a projection folds events into a
  * read model. Sealed traits, case classes, a trait with an implementation,
  * an object and a type alias - the shapes a Scala preview should show.
  */

type Offset = Long

sealed trait Event {
  def aggregateId: String
  def at: Long
}

final case class AccountOpened(aggregateId: String, owner: String, at: Long) extends Event
final case class MoneyDeposited(aggregateId: String, pennies: Long, at: Long) extends Event
final case class MoneyWithdrawn(aggregateId: String, pennies: Long, at: Long) extends Event
final case class AccountClosed(aggregateId: String, reason: String, at: Long) extends Event

final case class Stored(offset: Offset, event: Event)

sealed trait StreamError extends Product with Serializable
case object EmptyStream extends StreamError
final case class UnknownAggregate(id: String) extends StreamError
final case class OffsetTooOld(requested: Offset, earliest: Offset) extends StreamError

trait EventStore {
  def append(event: Event): Try[Stored]
  def read(from: Offset, limit: Int = 100): Either[StreamError, Seq[Stored]]
  def size: Int
}

final class InMemoryEventStore(private var events: Queue[Stored] = Queue.empty) extends EventStore {
  private var nextOffset: Offset = 0L

  override def append(event: Event): Try[Stored] = Try {
    val stored = Stored(nextOffset, event)
    events = events.enqueue(stored)
    nextOffset += 1
    stored
  }

  override def read(from: Offset, limit: Int = 100): Either[StreamError, Seq[Stored]] =
    if (events.isEmpty) Left(EmptyStream)
    else {
      val earliest = events.head.offset
      if (from < earliest) Left(OffsetTooOld(from, earliest))
      else Right(events.dropWhile(_.offset < from).take(limit).toSeq)
    }

  override def size: Int = events.size

  def compact(before: Offset): Int = {
    val (dropped, kept) = events.partition(_.offset < before)
    events = kept
    dropped.size
  }
}

final case class AccountState(
    id: String,
    owner: String,
    balancePennies: Long,
    closed: Boolean
) {
  def pounds: BigDecimal = BigDecimal(balancePennies) / 100
}

object AccountProjection {

  def empty(id: String): AccountState = AccountState(id, owner = "", balancePennies = 0L, closed = false)

  def apply(state: AccountState, event: Event): AccountState = event match {
    case AccountOpened(_, owner, _)     => state.copy(owner = owner)
    case MoneyDeposited(_, pennies, _)  => state.copy(balancePennies = state.balancePennies + pennies)
    case MoneyWithdrawn(_, pennies, _)  => state.copy(balancePennies = state.balancePennies - pennies)
    case AccountClosed(_, _, _)         => state.copy(closed = true)
  }

  def fold(id: String, events: Seq[Event]): AccountState =
    events.filter(_.aggregateId == id).foldLeft(empty(id))(apply)

  def foldAsync(id: String, store: EventStore)(implicit ec: ExecutionContext): Future[AccountState] =
    Future {
      store.read(0L) match {
        case Right(stored) => fold(id, stored.map(_.event))
        case Left(_)       => empty(id)
      }
    }
}

object EventStreamDemo {

  private def now(): Long = System.currentTimeMillis()

  def main(args: Array[String]): Unit = {
    val store = new InMemoryEventStore()

    val appended = Seq(
      AccountOpened("acc-1", "Ada Lovelace", now()),
      MoneyDeposited("acc-1", 250_00L, now()),
      MoneyWithdrawn("acc-1", 40_00L, now()),
      MoneyDeposited("acc-2", 10_00L, now())
    ).map(store.append)

    appended.collect { case Failure(problem) => println(s"append failed: $problem") }

    val state = AccountProjection.fold("acc-1", appended.collect { case Success(s) => s.event })
    println(s"${state.owner} holds ${state.pounds}")
    println(s"stored ${store.size} events")

    store.read(from = 99L) match {
      case Right(events) => println(s"read ${events.size}")
      case Left(problem) => println(s"cannot read: $problem")
    }
  }
}
