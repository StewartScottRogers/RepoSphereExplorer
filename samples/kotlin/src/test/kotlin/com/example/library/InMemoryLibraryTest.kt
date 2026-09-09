package com.example.library

import java.time.LocalDate
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.runTest

class InMemoryLibraryTest {

    private val dune = Book(
        isbn = Isbn("9780441013593"),
        title = "Dune",
        author = "Frank Herbert",
        copies = 1,
    )

    private fun library(vararg books: Book) = InMemoryLibrary(books.toList())

    @Test
    fun `a book nobody has borrowed is available`() = runTest {
        val library = library(dune)

        assertEquals(Availability.AVAILABLE, library.availability(dune.isbn))
    }

    @Test
    fun `an isbn the library has never held is not simply unavailable`() = runTest {
        val library = library(dune)

        assertEquals(Availability.UNKNOWN_BOOK, library.availability(Isbn("9780000000000")))
    }

    @Test
    fun `the last copy going out makes the book unavailable`() = runTest {
        val library = library(dune)

        library.lend(dune.isbn, memberId = 1)

        assertEquals(Availability.ALL_ON_LOAN, library.availability(dune.isbn))
    }

    @Test
    fun `lending the only copy twice is refused, not thrown`() = runTest {
        val library = library(dune)
        library.lend(dune.isbn, memberId = 1)

        val second = library.lend(dune.isbn, memberId = 2)

        assertIs<LoanResult.Refused>(second, "a refusal is an answer, not a failure")
    }

    @Test
    fun `the first loan is granted`() = runTest {
        val library = library(dune)

        assertIs<LoanResult.Granted>(library.lend(dune.isbn, memberId = 1))
    }

    @Test
    fun `a member's loans come back as a flow`() = runTest {
        val library = library(dune)
        library.lend(dune.isbn, memberId = 7)

        val loans = library.loans(memberId = 7).toList()

        assertEquals(1, loans.size)
    }

    @Test
    fun `nothing is overdue on the day it was borrowed`() = runTest {
        val library = library(dune)
        library.lend(dune.isbn, memberId = 1, today = LocalDate.of(2026, 1, 1))

        assertEquals(emptyList(), library.overdue(today = LocalDate.of(2026, 1, 1)))
    }
}
