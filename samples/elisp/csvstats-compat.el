;;; csvstats-compat.el --- Support for older Emacs

;; No lexical-binding header, as this file predates it. Every `let' here
;; is dynamically bound, and the closure below does not close over
;; anything. It is left that way so the file pane has something to warn
;; about.

;;; Code:

(require 'csvstats)

(defvar csvstats-compat--warned nil
  "Whether the one-time warning has been shown.")

(defun csvstats-compat-adapter (places)
  "Return a function reporting to PLACES decimal places."
  (lambda (n) (format "%.*f" places n)))

(defun csvstats-compat-check ()
  "Complain once if this Emacs is too old."
  (interactive)
  (unless csvstats-compat--warned
    (setq csvstats-compat--warned t)
    (message "csvstats wants Emacs 28.1 or later")))

(provide 'csvstats-compat)
;;; csvstats-compat.el ends here
