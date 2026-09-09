(ns ledger.core-test
  (:require [clojure.test :refer [deftest is testing]]
            [ledger.core :as ledger]))

(defn- posting [account amount]
  (ledger/posting account amount))

(deftest balanced-entries
  (testing "an entry whose postings sum to zero is balanced"
    (is (ledger/balanced? [(posting "assets:cash" 100)
                           (posting "income:sales" -100)])))

  (testing "an entry that does not sum to zero is not"
    (is (not (ledger/balanced? [(posting "assets:cash" 100)
                                (posting "income:sales" -99)])))))

(deftest accounts-are-listed-once
  (let [entry (ledger/entry "2026-01-01" "Sale"
                            [(posting "assets:cash" 100)
                             (posting "income:sales" -100)])]
    (is (= #{"assets:cash" "income:sales"} (set (ledger/accounts [entry]))))))

(deftest account-types-come-from-the-first-segment
  (testing "the type is the part before the first colon"
    (is (= :assets (ledger/account-type "assets:cash")))
    (is (= :income (ledger/account-type "income:sales")))))

(deftest balances-add-up
  (let [entries [(ledger/entry "2026-01-01" "Sale"
                               [(posting "assets:cash" 100)
                                (posting "income:sales" -100)])
                 (ledger/entry "2026-01-02" "Rent"
                               [(posting "assets:cash" -40)
                                (posting "expenses:rent" 40)])]]
    (testing "one account's balance is the sum of its postings"
      (is (= 60 (ledger/balance entries "assets:cash"))))

    (testing "a trial balance sums to zero, or the books are wrong"
      (is (zero? (reduce + (vals (ledger/trial-balance entries))))))))
