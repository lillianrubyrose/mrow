return (function(): MrowRoot
   return {
      init = function()
         if mrow.hostname == "nya" then
            require("hosts/nya")
         end
      end,
      aur_helper = "yay"
   };
end)();
