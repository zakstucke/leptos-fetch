const { scopedPreflightStyles } = require('tailwindcss-scoped-preflight');
const colors = require('tailwindcss/colors')

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./leptos-fetch/src/dev_tools/**/*.rs"],
  prefix:"lq-",
  theme: {
    extend: {
      colors: {
        'lq-background': colors.zinc[900],
        'lq-foreground': colors.zinc[100], 
        'lq-accent': colors.zinc[800], 
        'lq-border': colors.zinc[700],     

        'lq-input': colors.zinc[700],      
        'lq-input-foreground': colors.zinc[300] 
      },
    },
  },
   plugins: [
    require("@tailwindcss/forms")({
      strategy: 'class', 
    }),
    scopedPreflightStyles({
        cssSelector: '.leptos-fetch-devtools', 
        mode: 'matched only', 
    }),
],
}