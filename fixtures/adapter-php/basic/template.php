<?php
// A file that opens with markup and closes the PHP tag, which is the shape
// `LANGUAGE_PHP` exists for. Parsed with the code-only dialect this file is a
// syntax error, so it is here to keep that choice honest.
namespace App;

function render_title(string $title): string
{
    return $title;
}
?>
<h1><?= render_title("Reports") ?></h1>
