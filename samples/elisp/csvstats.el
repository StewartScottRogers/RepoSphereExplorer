;;; csvstats.el --- Summary statistics for a region -*- lexical-binding: t -*-

;; Copyright (C) 2026 The floor

;; Author: The floor <floor@example.com>
;; Version: 1.0
;; Package-Requires: ((emacs "28.1"))
;; Keywords: convenience, data
;; URL: https://example.com/floor/csvstats

;;; Commentary:

;; Select a region of comma-separated numbers and ask for its mean or
;; its standard deviation.
;;
;; `csvstats-deviation' deliberately has no autoload cookie, so it
;; cannot be run by name until something else has loaded this file. It
;; is left that way so the file pane has something to warn about.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(defgroup csvstats nil
  "Summary statistics for a region of numbers."
  :group 'tools
  :prefix "csvstats-")

(defcustom csvstats-separator ","
  "What separates one field from the next."
  :type 'string
  :group 'csvstats)

(defcustom csvstats-decimal-places 3
  "How many decimal places to report."
  :type 'integer
  :group 'csvstats)

(defvar csvstats--last-result nil
  "The last figure reported, for `csvstats-repeat'.")

(defconst csvstats--number-pattern "-?[0-9]+\\(\\.[0-9]+\\)?"
  "What counts as a number in a region.")

(defun csvstats--numbers (beginning end)
  "Return the numbers between BEGINNING and END as a list."
  (let ((text (buffer-substring-no-properties beginning end))
        (found nil))
    (dolist (field (split-string text csvstats-separator t))
      (when (string-match-p csvstats--number-pattern (string-trim field))
        (push (string-to-number field) found)))
    (nreverse found)))

(defun csvstats--average (numbers)
  "Return the mean of NUMBERS, or nil when there are none."
  (when numbers
    (/ (apply #'+ numbers) (float (length numbers)))))

(defun csvstats--spread (numbers)
  "Return the population standard deviation of NUMBERS."
  (when-let ((mean (csvstats--average numbers)))
    (sqrt (/ (apply #'+ (mapcar (lambda (n) (expt (- n mean) 2)) numbers))
             (float (length numbers))))))

;;;###autoload
(defun csvstats-mean (beginning end)
  "Report the mean of the numbers between BEGINNING and END."
  (interactive "r")
  (let ((mean (csvstats--average (csvstats--numbers beginning end))))
    (setq csvstats--last-result mean)
    (message "mean: %.*f" csvstats-decimal-places (or mean 0))))

(defun csvstats-deviation (beginning end)
  "Report the standard deviation between BEGINNING and END."
  (interactive "r")
  (let ((spread (csvstats--spread (csvstats--numbers beginning end))))
    (setq csvstats--last-result spread)
    (message "deviation: %.*f" csvstats-decimal-places (or spread 0))))

(defvar csvstats-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "C-c s m") #'csvstats-mean)
    (define-key map (kbd "C-c s d") #'csvstats-deviation)
    map)
  "Keymap for `csvstats-mode'.")

;;;###autoload
(define-minor-mode csvstats-mode
  "Report summary statistics for a selected region."
  :lighter " CSV"
  :keymap csvstats-mode-map
  :group 'csvstats)

(provide 'csvstats)
;;; csvstats.el ends here
