package com.example.library

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.withContext
import java.time.LocalDate
import java.util.concurrent.ConcurrentHashMap

/**
 * A lending library: books, members and loans, with the repository behind
 * a coroutine-friendly interface and every failure modelled as a sealed
 * result rather than an exception.
 */

@JvmInline
value class Isbn(val value: String) {
    init {
        require(value.length in 10..17) { "not an ISBN: $value" }
    }
}

data class Book(
    val isbn: Isbn,
    val title: String,
    val author: String,
    val copies: Int,
) {
    val shortTitle: String
        get() = if (title.length <= 40) title else title.take(37) + "..."
}

data class Member(val id: Long, val name: String, val joined: LocalDate)

data class Loan(
    val isbn: Isbn,
    val memberId: Long,
    val takenOn: LocalDate,
    val dueOn: LocalDate = takenOn.plusWeeks(3),
) {
    fun isOverdue(today: LocalDate = LocalDate.now()): Boolean = today > dueOn
}

enum class Availability { AVAILABLE, ALL_ON_LOAN, UNKNOWN_BOOK }

sealed interface LoanResult {
    data class Granted(val loan: Loan) : LoanResult
    data class Refused(val reason: String) : LoanResult
    data object AlreadyHeld : LoanResult
}

interface LibraryRepository {
    suspend fun book(isbn: Isbn): Book?
    suspend fun save(book: Book)
    fun loans(memberId: Long): Flow<Loan>
}

class InMemoryLibrary(
    seed: Collection<Book> = emptyList(),
) : LibraryRepository {

    private val books = ConcurrentHashMap<String, Book>()
    private val loans = mutableListOf<Loan>()

    init {
        seed.forEach { books[it.isbn.value] = it }
    }

    override suspend fun book(isbn: Isbn): Book? = withContext(Dispatchers.IO) {
        books[isbn.value]
    }

    override suspend fun save(book: Book) = withContext(Dispatchers.IO) {
        books[book.isbn.value] = book
    }

    override fun loans(memberId: Long): Flow<Loan> = flow {
        loans.filter { it.memberId == memberId }.forEach { emit(it) }
    }

    suspend fun availability(isbn: Isbn): Availability {
        val book = book(isbn) ?: return Availability.UNKNOWN_BOOK
        val out = loans.count { it.isbn == isbn }
        return if (out < book.copies) Availability.AVAILABLE else Availability.ALL_ON_LOAN
    }

    suspend fun lend(isbn: Isbn, memberId: Long, today: LocalDate = LocalDate.now()): LoanResult {
        if (loans.any { it.isbn == isbn && it.memberId == memberId }) {
            return LoanResult.AlreadyHeld
        }
        return when (availability(isbn)) {
            Availability.UNKNOWN_BOOK -> LoanResult.Refused("no such book: ${isbn.value}")
            Availability.ALL_ON_LOAN -> LoanResult.Refused("every copy is out")
            Availability.AVAILABLE -> Loan(isbn, memberId, today)
                .also { loans += it }
                .let(LoanResult::Granted)
        }
    }

    fun overdue(today: LocalDate = LocalDate.now()): List<Loan> = loans.filter { it.isOverdue(today) }
}

fun Collection<Book>.byAuthor(): Map<String, List<Book>> = groupBy(Book::author)

suspend fun main() {
    val library = InMemoryLibrary(
        listOf(
            Book(Isbn("9780262033848"), "Introduction to Algorithms", "Cormen", copies = 2),
            Book(Isbn("9781593278281"), "The Rust Programming Language", "Klabnik", copies = 1),
        ),
    )

    when (val result = library.lend(Isbn("9781593278281"), memberId = 7)) {
        is LoanResult.Granted -> println("due ${result.loan.dueOn}")
        is LoanResult.Refused -> println("refused: ${result.reason}")
        LoanResult.AlreadyHeld -> println("already held")
    }

    println(library.availability(Isbn("9781593278281")))
}
