(ns ledger.core
  "A double-entry ledger: postings must balance, accounts are derived,
   and a running balance is a reduction rather than mutable state."
  (:require [clojure.string :as str]
            [clojure.set :as set])
  (:import (java.time LocalDate)))

(def ^:private account-types
  #{:asset :liability :equity :income :expense})

(defrecord Posting [account amount])

(defrecord Entry [date description postings])

(defn posting
  "A single leg of an entry. Debits are positive, credits negative."
  [account amount]
  (->Posting account (bigdec amount)))

(defn balanced?
  "An entry balances when its postings sum to zero."
  [entry]
  (zero? (reduce + 0M (map :amount (:postings entry)))))

(defn entry
  "Build an entry, refusing one that does not balance."
  [date description & postings]
  (let [candidate (->Entry date description (vec postings))]
    (if (balanced? candidate)
      candidate
      (throw (ex-info "entry does not balance"
                      {:description description
                       :difference (reduce + 0M (map :amount postings))})))))

(defn accounts
  "Every account named by a sequence of entries, sorted."
  [entries]
  (->> entries
       (mapcat :postings)
       (map :account)
       (into (sorted-set))))

(defn account-type
  "The type of an account, taken from the first segment of its name."
  [account]
  (let [head (keyword (first (str/split (name account) #":")))]
    (if (contains? account-types head) head :unknown)))

(defn balance
  "The balance of one account across entries."
  [entries account]
  (->> entries
       (mapcat :postings)
       (filter #(= account (:account %)))
       (map :amount)
       (reduce + 0M)))

(defn trial-balance
  "Every account with its balance, as a sorted map."
  [entries]
  (into (sorted-map)
        (for [account (accounts entries)]
          [account (balance entries account)])))

(defn running-balance
  "A lazy sequence of [entry balance-after] for one account."
  [entries account]
  (->> entries
       (reductions
        (fn [[_ total] current]
          [current (+ total (balance [current] account))])
        [nil 0M])
       (drop 1)))

(defn unbalanced
  "The entries that do not balance, for a file loaded from elsewhere."
  [entries]
  (remove balanced? entries))

(def sample-entries
  [(entry (LocalDate/parse "2026-01-04") "opening balance"
          (posting :asset:checking 2500.00M)
          (posting :equity:opening -2500.00M))
   (entry (LocalDate/parse "2026-01-11") "monthly rent"
          (posting :expense:rent 1200.00M)
          (posting :asset:checking -1200.00M))
   (entry (LocalDate/parse "2026-01-28") "invoice 214"
          (posting :asset:checking 3100.00M)
          (posting :income:consulting -3100.00M))])

(defn -main
  "Print a trial balance for the sample ledger."
  [& _args]
  (doseq [[account amount] (trial-balance sample-entries)]
    (println (format "%-22s %12s %s"
                     (name account)
                     (str amount)
                     (name (account-type account)))))
  (println "accounts:" (count (accounts sample-entries)))
  (println "unbalanced:" (count (unbalanced sample-entries))))
