      ******************************************************************
      * CSVSTATS - summary statistics for a file of readings.
      *
      * REPORT-DEVIATION is never performed and the paragraph above it
      * ends with STOP RUN, so nothing reaches it. It is left that way
      * so the file pane has something to warn about.
      ******************************************************************
       IDENTIFICATION DIVISION.
       PROGRAM-ID. CSVSTATS.
       AUTHOR. THE FLOOR.

       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT SAMPLE-FILE ASSIGN TO "samples.dat"
               ORGANIZATION IS LINE SEQUENTIAL
               FILE STATUS IS WS-FILE-STATUS.

       DATA DIVISION.
       FILE SECTION.
       FD  SAMPLE-FILE.
       01  SAMPLE-RECORD.
           05  SR-NAME           PIC X(20).
           05  SR-VALUE          PIC S9(7)V99.

       WORKING-STORAGE SECTION.
       01  WS-FILE-STATUS        PIC XX    VALUE SPACES.
       01  WS-EOF-FLAG           PIC X     VALUE "N".
           88  WS-EOF                      VALUE "Y".
       01  WS-TOTALS.
           05  WS-COUNT          PIC 9(6)  VALUE ZERO.
           05  WS-SUM            PIC S9(9)V99 VALUE ZERO.
           05  WS-SUM-SQUARES    PIC S9(13)V99 VALUE ZERO.
       77  WS-MEAN               PIC S9(7)V99 VALUE ZERO.
       77  WS-DEVIATION          PIC S9(7)V99 VALUE ZERO.

       PROCEDURE DIVISION.
       MAIN-PARAGRAPH.
           PERFORM OPEN-FILES
           PERFORM READ-SAMPLES UNTIL WS-EOF
           PERFORM CLOSE-FILES
           PERFORM COMPUTE-MEAN
           PERFORM REPORT-RESULTS
           STOP RUN.

       OPEN-FILES.
           OPEN INPUT SAMPLE-FILE.

       READ-SAMPLES.
           READ SAMPLE-FILE
               AT END
                   MOVE "Y" TO WS-EOF-FLAG
               NOT AT END
                   ADD 1 TO WS-COUNT
                   ADD SR-VALUE TO WS-SUM
           END-READ.

       CLOSE-FILES.
           CLOSE SAMPLE-FILE.

       COMPUTE-MEAN.
           IF WS-COUNT > ZERO
               COMPUTE WS-MEAN = WS-SUM / WS-COUNT
           END-IF.

       REPORT-RESULTS.
           DISPLAY "COUNT: " WS-COUNT
           DISPLAY "MEAN:  " WS-MEAN.

       REPORT-DEVIATION.
           DISPLAY "DEVIATION: " WS-DEVIATION.
