;;; matrix.scm -- small matrix and vector routines in R7RS Scheme.
;;;
;;; Matrices are vectors of row vectors. Everything is written with
;;; explicit recursion or named let, which is what a Scheme preview
;;; should show: define, define-record-type, let-syntax and a few
;;; higher-order helpers.

(define-record-type <matrix>
  (make-matrix rows columns cells)
  matrix?
  (rows matrix-rows)
  (columns matrix-columns)
  (cells matrix-cells))

(define (matrix-create rows columns fill)
  (make-matrix rows columns
               (let loop ((row 0) (acc '()))
                 (if (= row rows)
                     (list->vector (reverse acc))
                     (loop (+ row 1)
                           (cons (make-vector columns fill) acc))))))

(define (matrix-ref m row column)
  (vector-ref (vector-ref (matrix-cells m) row) column))

(define (matrix-set! m row column value)
  (vector-set! (vector-ref (matrix-cells m) row) column value))

(define (matrix-identity size)
  (let ((m (matrix-create size size 0)))
    (let loop ((index 0))
      (if (= index size)
          m
          (begin
            (matrix-set! m index index 1)
            (loop (+ index 1)))))))

(define (matrix-from-lists rows)
  (let* ((row-count (length rows))
         (column-count (length (car rows)))
         (m (matrix-create row-count column-count 0)))
    (let loop ((row 0) (remaining rows))
      (if (null? remaining)
          m
          (begin
            (let inner ((column 0) (values (car remaining)))
              (unless (null? values)
                (matrix-set! m row column (car values))
                (inner (+ column 1) (cdr values))))
            (loop (+ row 1) (cdr remaining)))))))

(define (matrix-map procedure m)
  (let ((out (matrix-create (matrix-rows m) (matrix-columns m) 0)))
    (let loop ((row 0))
      (if (= row (matrix-rows m))
          out
          (begin
            (let inner ((column 0))
              (when (< column (matrix-columns m))
                (matrix-set! out row column (procedure (matrix-ref m row column)))
                (inner (+ column 1))))
            (loop (+ row 1)))))))

(define (matrix-transpose m)
  (let ((out (matrix-create (matrix-columns m) (matrix-rows m) 0)))
    (let loop ((row 0))
      (if (= row (matrix-rows m))
          out
          (begin
            (let inner ((column 0))
              (when (< column (matrix-columns m))
                (matrix-set! out column row (matrix-ref m row column))
                (inner (+ column 1))))
            (loop (+ row 1)))))))

(define (matrix-multiply a b)
  (unless (= (matrix-columns a) (matrix-rows b))
    (error "inner dimensions disagree" (matrix-columns a) (matrix-rows b)))
  (let ((out (matrix-create (matrix-rows a) (matrix-columns b) 0)))
    (let loop ((row 0))
      (if (= row (matrix-rows a))
          out
          (begin
            (let column-loop ((column 0))
              (when (< column (matrix-columns b))
                (matrix-set! out row column
                             (let sum ((k 0) (total 0))
                               (if (= k (matrix-columns a))
                                   total
                                   (sum (+ k 1)
                                        (+ total (* (matrix-ref a row k)
                                                    (matrix-ref b k column)))))))
                (column-loop (+ column 1))))
            (loop (+ row 1)))))))

(define (matrix-trace m)
  (let loop ((index 0) (total 0))
    (if (= index (min (matrix-rows m) (matrix-columns m)))
        total
        (loop (+ index 1) (+ total (matrix-ref m index index))))))

(define (matrix->lists m)
  (let loop ((row 0) (acc '()))
    (if (= row (matrix-rows m))
        (reverse acc)
        (loop (+ row 1)
              (cons (vector->list (vector-ref (matrix-cells m) row)) acc)))))

(define (display-matrix m)
  (for-each (lambda (row) (display row) (newline))
            (matrix->lists m)))

(define sample (matrix-from-lists '((1 2) (3 4))))

(display-matrix (matrix-multiply sample (matrix-identity 2)))
(display "trace: ")
(display (matrix-trace sample))
(newline)
(display-matrix (matrix-map (lambda (value) (* value value)) sample))
